#![no_std]

extern crate alloc;

use alloc::format;

#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

use asr::{
    future::next_tick,
    game_engine::unity::{
        mono::{
            Class,
            Module,
        },
        scene_manager::SceneManager,
    },
    timer,
    Address,
    PointerSize,
    Process,
};

use time::Duration;

asr::async_main!(stable);
asr::panic_handler!();


// ============================================================================
// Configuration
// ============================================================================

// --------------------------------------------------------------------------
// Stage 1決定時のFadeIn補正
//
// StageSelectManager側:
//
// fade.FadeIn(1f, delegate
// {
//     SceneManager.LoadScene(sceneName[index]);
// });
//
// のため、BattleManager検出時点では
// 本来のRTA開始点から約1秒経過している。
//
// まずは1000msで運用し、後で実測補正可能。
// --------------------------------------------------------------------------

const START_TIME_CORRECTION_MS: i64 = 1000;


// --------------------------------------------------------------------------
// finishFlg 0→1の瞬間だけvideo読取が失敗した場合の再試行回数。
// 通常は同tickで成功する想定。
// --------------------------------------------------------------------------

const FINISH_VIDEO_RETRY_TICKS: u8 = 30;


// ============================================================================
// Unity / Mono Layout
// ============================================================================

// UnityEngine.Objectのmanaged wrapperにあるnative pointer。
//
// 今回の64bit Mono環境では +0x10。
// native objectの生存確認に使用。
const UNITY_OBJECT_CACHED_PTR: u64 = 0x10;


// --------------------------------------------------------------------------
// Unity 2023.2.20f1 native Hierarchy layout
//
// 実測結果:
// Scene +0xE8 / +0xF0 -> root linked list
// list node +0x10      -> Transform
// Component +0x20      -> GameObject
// GameObject +0x20     -> component array
// GameObject +0x50     -> name C string
// native Component +0x18 -> scripting object handle
// scripting object handle -> managed MonoObject (1 dereference)
//
// root list / component arrayはいずれも今回のゲームで複数回再現確認済み。
// --------------------------------------------------------------------------

const UNITY2023_SCENE_ROOT_A: u64 = 0xE8;
const UNITY2023_SCENE_ROOT_B: u64 = 0xF0;

const UNITY2023_ROOT_NODE_TRANSFORM: u64 = 0x10;

const UNITY2023_COMPONENT_GAME_OBJECT: u64 = 0x20;
const UNITY2023_GAME_OBJECT_COMPONENTS: u64 = 0x20;
const UNITY2023_GAME_OBJECT_NAME: u64 = 0x50;

const UNITY2023_SCRIPTING_OBJECT_HANDLE: u64 = 0x18;

// Component arrayは
// +0x08, +0x18, +0x28 ... にComponent pointerが並ぶ。
const UNITY2023_COMPONENT_PAIR_FIRST: u64 = 0x08;
const UNITY2023_COMPONENT_PAIR_STRIDE: u64 = 0x10;
const UNITY2023_COMPONENT_SCAN_COUNT: u32 = 16;

const UNITY2023_ROOT_LIST_MAX_NODES: u32 = 128;


// ============================================================================
// Basic Pointer Helpers
// ============================================================================

fn read_non_null_pointer(
    process: &Process,
    address: Address,
    pointer_size: PointerSize,
) -> Option<Address> {
    process
        .read_pointer(
            address,
            pointer_size,
        )
        .ok()
        .filter(
            |address|
                !address.is_null()
        )
}


// ============================================================================
// UnityEngine.Object Native State
// ============================================================================

// --------------------------------------------------------------------------
// Some(true)
//     native objectが存在
//
// Some(false)
//     native objectがnull / destroyed
//
// None
//     メモリ読取失敗
// --------------------------------------------------------------------------

fn unity_native_alive(
    process: &Process,
    managed_object: Address,
    pointer_size: PointerSize,
) -> Option<bool> {
    process
        .read_pointer(
            managed_object
                + UNITY_OBJECT_CACHED_PTR,
            pointer_size,
        )
        .ok()
        .map(
            |native|
                !native.is_null()
        )
}


// --------------------------------------------------------------------------
// BattleManager内のUnityEngine.Object参照について
// native objectが生存しているか調べる。
//
// 今回は BattleManager.video 判定用。
// --------------------------------------------------------------------------

fn unity_reference_alive(
    process: &Process,
    owner: Address,
    field_offset: u32,
    pointer_size: PointerSize,
) -> Option<bool> {
    let managed_reference =
        process
            .read_pointer(
                owner
                    + field_offset as u64,
                pointer_size,
            )
            .ok()?;


    // managed reference自体がnull
    if managed_reference.is_null() {
        return Some(false);
    }


    unity_native_alive(
        process,
        managed_reference,
        pointer_size,
    )
}


// ============================================================================
// Mono Field Offset Helper
// ============================================================================

async fn get_field_offset_logged(
    process: &Process,
    mono: &Module,
    class: &Class,
    class_name: &str,
    field_name: &str,
) -> u32 {
    asr::print_message(
        &format!(
            "Resolving {}.{} ...",
            class_name,
            field_name
        )
    );


    let offset =
        class
            .wait_get_field_offset(
                process,
                mono,
                field_name,
            )
            .await;


    asr::print_message(
        &format!(
            "{}.{} = 0x{:X}",
            class_name,
            field_name,
            offset
        )
    );


    offset
}


// ============================================================================
// BattleManager Values
// ============================================================================

fn read_battle_values(
    process: &Process,
    address: Address,
    stage_num_offset: u32,
    start_flg_offset: u32,
    finish_flg_offset: u32,
    pointer_size: PointerSize,
) -> Option<(
    i32,
    u8,
    u8,
)> {
    // ------------------------------------------------------------
    // native BattleManagerが明確にdestroy済みなら不採用。
    // ------------------------------------------------------------

    if unity_native_alive(
        process,
        address,
        pointer_size,
    ) == Some(false)
    {
        return None;
    }


    // ------------------------------------------------------------
    // stageNum
    // ------------------------------------------------------------

    let stage_num =
        process
            .read::<i32>(
                address
                    + stage_num_offset as u64
            )
            .ok()?;


    // ------------------------------------------------------------
    // startFlg
    // ------------------------------------------------------------

    let start_flg =
        process
            .read::<u8>(
                address
                    + start_flg_offset as u64
            )
            .ok()?;


    // ------------------------------------------------------------
    // finishFlg
    // ------------------------------------------------------------

    let finish_flg =
        process
            .read::<u8>(
                address
                    + finish_flg_offset as u64
            )
            .ok()?;


    // ------------------------------------------------------------
    // Validation
    // ------------------------------------------------------------

    if !(0..=20)
        .contains(&stage_num)
    {
        return None;
    }


    if start_flg > 1 {
        return None;
    }


    if finish_flg > 1 {
        return None;
    }


    Some((
        stage_num,
        start_flg,
        finish_flg,
    ))
}


// ============================================================================
// Unity 2023.2.20f1 Hierarchy BattleManager Resolver
// ============================================================================
//
// ASR SceneManagerのcurrent Scene取得だけを利用し、
// Unity 2023.2.20f1で実測したnative Hierarchy layoutを手動で辿る。
//
// Scene
//   -> root linked list
//   -> Transform
//   -> GameObject "BattleManager"
//   -> Component array
//   -> native MonoBehaviour Component
//   -> scripting object handle
//   -> managed BattleManager
//
// managed object候補は、
//   ・managed +0x10 が元のnative Componentへ戻る
//   ・stageNum / startFlg / finishFlgが既存validationを通る
// の2条件で確定する。
// ============================================================================

fn is_battle_manager_cstr(
    process: &Process,
    address: Address,
) -> bool {
    if address.is_null() {
        return false;
    }

    match process.read::<[u8; 14]>(address) {
        Ok(bytes) => {
            bytes == *b"BattleManager\0"
        }

        Err(_) => {
            false
        }
    }
}


fn resolve_managed_battle_from_game_object(
    process: &Process,
    game_object: Address,
    stage_num_offset: u32,
    start_flg_offset: u32,
    finish_flg_offset: u32,
    pointer_size: PointerSize,
) -> Option<(Address, i32, u8, u8)> {
    let component_array =
        read_non_null_pointer(
            process,
            game_object
                + UNITY2023_GAME_OBJECT_COMPONENTS,
            pointer_size,
        )?;


    let mut index:
        u32 = 0;


    while index
        < UNITY2023_COMPONENT_SCAN_COUNT
    {
        let pair_offset =
            UNITY2023_COMPONENT_PAIR_FIRST
                + index as u64
                    * UNITY2023_COMPONENT_PAIR_STRIDE;


        if let Some(native_component) =
            read_non_null_pointer(
                process,
                component_array + pair_offset,
                pointer_size,
            )
        {
            // native ComponentがこのGameObjectに属していることを確認。
            let owner_matches =
                read_non_null_pointer(
                    process,
                    native_component
                        + UNITY2023_COMPONENT_GAME_OBJECT,
                    pointer_size,
                )
                .is_some_and(
                    |owner|
                        owner.value()
                            == game_object.value()
                );


            if owner_matches {
                if let Some(scripting_handle) =
                    read_non_null_pointer(
                        process,
                        native_component
                            + UNITY2023_SCRIPTING_OBJECT_HANDLE,
                        pointer_size,
                    )
                {
                    if let Some(managed_object) =
                        read_non_null_pointer(
                            process,
                            scripting_handle,
                            pointer_size,
                        )
                    {
                        // managed UnityEngine.Object +0x10 が
                        // 元のnative Componentに戻ることを確認。
                        let native_back_matches =
                            read_non_null_pointer(
                                process,
                                managed_object
                                    + UNITY_OBJECT_CACHED_PTR,
                                pointer_size,
                            )
                            .is_some_and(
                                |native_back|
                                    native_back.value()
                                        == native_component.value()
                            );


                        if native_back_matches {
                            if let Some((
                                stage_num,
                                start_flg,
                                finish_flg,
                            )) =
                                read_battle_values(
                                    process,
                                    managed_object,
                                    stage_num_offset,
                                    start_flg_offset,
                                    finish_flg_offset,
                                    pointer_size,
                                )
                            {
                                return Some((
                                    managed_object,
                                    stage_num,
                                    start_flg,
                                    finish_flg,
                                ));
                            }
                        }
                    }
                }
            }
        }


        index += 1;
    }


    None
}


fn resolve_battle_from_root_list(
    process: &Process,
    scene_address: Address,
    scene_root_offset: u64,
    link_offset: u64,
    stage_num_offset: u32,
    start_flg_offset: u32,
    finish_flg_offset: u32,
    pointer_size: PointerSize,
) -> Option<(Address, i32, u8, u8)> {
    let start =
        read_non_null_pointer(
            process,
            scene_address + scene_root_offset,
            pointer_size,
        )?;


    let mut current =
        start;

    let mut depth:
        u32 = 0;


    while depth
        < UNITY2023_ROOT_LIST_MAX_NODES
    {
        let transform =
            read_non_null_pointer(
                process,
                current
                    + UNITY2023_ROOT_NODE_TRANSFORM,
                pointer_size,
            );


        if let Some(transform) =
            transform
        {
            if let Some(game_object) =
                read_non_null_pointer(
                    process,
                    transform
                        + UNITY2023_COMPONENT_GAME_OBJECT,
                    pointer_size,
                )
            {
                if let Some(name_pointer) =
                    read_non_null_pointer(
                        process,
                        game_object
                            + UNITY2023_GAME_OBJECT_NAME,
                        pointer_size,
                    )
                {
                    if is_battle_manager_cstr(
                        process,
                        name_pointer,
                    ) {
                        if let Some(result) =
                            resolve_managed_battle_from_game_object(
                                process,
                                game_object,
                                stage_num_offset,
                                start_flg_offset,
                                finish_flg_offset,
                                pointer_size,
                            )
                        {
                            return Some(result);
                        }
                    }
                }
            }
        }


        let next =
            match read_non_null_pointer(
                process,
                current + link_offset,
                pointer_size,
            ) {
                Some(address) => {
                    address
                }

                None => {
                    break;
                }
            };


        if next.value()
            == current.value()
        {
            break;
        }


        if depth > 0
            && next.value()
                == start.value()
        {
            break;
        }


        current =
            next;

        depth += 1;
    }


    None
}


fn resolve_battle_manager_unity2023(
    process: &Process,
    scene_manager: &SceneManager,
    stage_num_offset: u32,
    start_flg_offset: u32,
    finish_flg_offset: u32,
    pointer_size: PointerSize,
) -> Option<(Address, i32, u8, u8)> {
    let scene =
        scene_manager
            .get_current_scene(process)
            .ok()?;


    let scene_address =
        scene.address();


    // 実測では4通りすべてBattleManagerへ到達できた。
    // まずE8/+0x00を主経路とし、残りをfallbackにする。
    const ROOT_PATHS:
        &[(u64, u64)] =
        &[
            (UNITY2023_SCENE_ROOT_A, 0x00),
            (UNITY2023_SCENE_ROOT_A, 0x08),
            (UNITY2023_SCENE_ROOT_B, 0x00),
            (UNITY2023_SCENE_ROOT_B, 0x08),
        ];


    for &(scene_root_offset, link_offset)
        in ROOT_PATHS
    {
        if let Some(result) =
            resolve_battle_from_root_list(
                process,
                scene_address,
                scene_root_offset,
                link_offset,
                stage_num_offset,
                start_flg_offset,
                finish_flg_offset,
                pointer_size,
            )
        {
            return Some(result);
        }
    }


    None
}


// ============================================================================
// Active BattleManager Resolver (Unity 2023.2 Hierarchy)
// ============================================================================
//
// 通常はHierarchyから現在のBattleManagerを毎tick解決する。
// Scene取得が一時的に失敗したtickだけは、直前のmanaged objectが
// まだ生存していればcached addressをfallbackとして使用する。
//
// Scene変更時はcurrent_battleをNoneへ戻すため、旧SceneのManagerを
// fallbackとして誤使用しない。
// ============================================================================

fn resolve_active_battle_manager_unity2023(
    process: &Process,
    scene_manager: &SceneManager,
    preferred: Option<Address>,
    stage_num_offset: u32,
    start_flg_offset: u32,
    finish_flg_offset: u32,
    pointer_size: PointerSize,
) -> Option<(Address, i32, u8, u8)> {
    // ------------------------------------------------------------
    // 1. Primary: Unity 2023.2 Hierarchy
    // ------------------------------------------------------------

    if let Some(result) =
        resolve_battle_manager_unity2023(
            process,
            scene_manager,
            stage_num_offset,
            start_flg_offset,
            finish_flg_offset,
            pointer_size,
        )
    {
        return Some(result);
    }


    // ------------------------------------------------------------
    // 2. Fallback: cached managed BattleManager
    // ------------------------------------------------------------

    if let Some(address) = preferred {
        if let Some((
            stage_num,
            start_flg,
            finish_flg,
        )) =
            read_battle_values(
                process,
                address,
                stage_num_offset,
                start_flg_offset,
                finish_flg_offset,
                pointer_size,
            )
        {
            return Some((
                address,
                stage_num,
                start_flg,
                finish_flg,
            ));
        }
    }


    None
}


// ============================================================================
// Main
// ============================================================================

async fn main() {
    asr::print_message(
        "ScreenSaver BATTLE Auto Splitter started."
    );


    loop {
        // ==================================================================
        // Process
        // ==================================================================

        let process =
            Process::wait_attach(
                "ScreensaverBATTLE.exe"
            )
            .await;


        asr::print_message(
            "Attached to ScreensaverBATTLE.exe"
        );


        process
            .until_closes(async {
                // ==========================================================
                // Unity Mono
                // ==========================================================

                let mono =
                    Module::wait_attach_auto_detect(
                        &process
                    )
                    .await;


                asr::print_message(
                    "Attached to Unity Mono."
                );


                let pointer_size =
                    mono.get_pointer_size();


                if !matches!(
                    pointer_size,
                    PointerSize::Bit64
                ) {
                    asr::print_message(
                        "ERROR: Expected 64-bit process."
                    );


                    loop {
                        next_tick().await;
                    }
                }


                // ==========================================================
                // Assembly-CSharp
                // ==========================================================

                let image =
                    mono
                        .wait_get_default_image(
                            &process
                        )
                        .await;


                let battle_class =
                    image
                        .wait_get_class(
                            &process,
                            &mono,
                            "BattleManager",
                        )
                        .await;


                asr::print_message(
                    "Found BattleManager class."
                );


                // ==========================================================
                // Field Offsets
                // ==========================================================

                let stage_num_offset =
                    get_field_offset_logged(
                        &process,
                        &mono,
                        &battle_class,
                        "BattleManager",
                        "stageNum",
                    )
                    .await;


                let start_flg_offset =
                    get_field_offset_logged(
                        &process,
                        &mono,
                        &battle_class,
                        "BattleManager",
                        "startFlg",
                    )
                    .await;


                let finish_flg_offset =
                    get_field_offset_logged(
                        &process,
                        &mono,
                        &battle_class,
                        "BattleManager",
                        "finishFlg",
                    )
                    .await;


                let video_offset =
                    get_field_offset_logged(
                        &process,
                        &mono,
                        &battle_class,
                        "BattleManager",
                        "video",
                    )
                    .await;


                asr::print_message(
                    "BattleManager fields ready."
                );


                // ==========================================================
                // SceneManager
                //
                // Scene index変更検出と
                // Unity 2023.2 Hierarchy探索のScene取得に使用。
                // ==========================================================

                let scene_manager =
                    SceneManager::wait_attach(
                        &process
                    )
                    .await;


                asr::print_message(
                    "SceneManager attached."
                );


                asr::print_message(
                    "Auto Splitter ready (Unity2023 Hierarchy only)."
                );


                // ==========================================================
                // State
                // ==========================================================

                // ----------------------------------------------------------
                // Scene
                // ----------------------------------------------------------

                let mut last_scene_index:
                    Option<i32> = None;


                // 別Sceneへ移動したあと、
                // 新SceneのBattleManagerを取得するまで保持。
                //
                // Retryは同一Scene indexなのでtrueにならない。
                let mut scene_transition_pending:
                    bool = false;


                // ----------------------------------------------------------
                // BattleManager
                // ----------------------------------------------------------

                let mut current_battle:
                    Option<Address> = None;

                let mut last_stage_num:
                    Option<i32> = None;


                let mut last_start_flg:
                    Option<u8> = None;


                let mut last_finish_flg:
                    Option<u8> = None;


                // ----------------------------------------------------------
                // finish時のvideo再試行
                // ----------------------------------------------------------

                let mut pending_finish_video_check:
                    u8 = 0;


                let mut finish_battle_address:
                    Option<Address> = None;


                // ==========================================================
                // Main Loop
                // ==========================================================

                loop {
                    // ======================================================
                    // 1. Scene Transition
                    // ======================================================

                    if let Ok(scene_index) =
                        scene_manager
                            .get_current_scene_index(
                                &process
                            )
                    {
                        // 初期化中の -1 等は無視。
                        if scene_index >= 0 {
                            match last_scene_index {
                                // ------------------------------------------
                                // First scene
                                // ------------------------------------------

                                None => {
                                    last_scene_index =
                                        Some(scene_index);
                                }


                                // ------------------------------------------
                                // Scene changed
                                // ------------------------------------------

                                Some(previous)
                                    if previous
                                        != scene_index =>
                                {
                                    asr::print_message(
                                        &format!(
                                            "Scene changed: {} -> {}",
                                            previous,
                                            scene_index
                                        )
                                    );


                                    last_scene_index =
                                        Some(scene_index);


                                    // 次に現れるBattleManagerで
                                    // Stage 1かどうか判断。
                                    scene_transition_pending =
                                        true;


                                    // 旧SceneのManagerを捨てる。
                                    current_battle =
                                        None;

                                    last_stage_num =
                                        None;


                                    last_start_flg =
                                        None;


                                    last_finish_flg =
                                        None;


                                    pending_finish_video_check =
                                        0;


                                    finish_battle_address =
                                        None;
                                }


                                _ => {}
                            }
                        }
                    }
                    // ======================================================
                    // 2. Resolve Current BattleManager
                    //
                    // Unity 2023.2.20f1で実測したHierarchy layoutを使用。
                    // 固定mono module offset / A600 / A540 / B600 / B540は
                    // ここでは一切使用しない。
                    // ======================================================

                    let battle_result =
                        resolve_active_battle_manager_unity2023(
                            &process,
                            &scene_manager,
                            current_battle,
                            stage_num_offset,
                            start_flg_offset,
                            finish_flg_offset,
                            pointer_size,
                        );


                    if let Some((
                        battle_address,
                        stage_num,
                        start_flg,
                        finish_flg,
                    )) = battle_result
                    {
                        // ==================================================
                        // BattleManager Instance Changed
                        // ==================================================

                        let battle_changed =
                            match current_battle {
                                Some(old) => {
                                    old.value()
                                        != battle_address.value()
                                }

                                None => {
                                    true
                                }
                            };


                        if battle_changed {
                            asr::print_message(
                                &format!(
                                    "BattleManager = 0x{:X} (Unity2023 Hierarchy)",
                                    battle_address.value()
                                )
                            );


                            current_battle =
                                Some(battle_address);


                            // 新しいManagerなので
                            // edge detectionを初期化。
                            last_stage_num =
                                None;


                            last_start_flg =
                                None;


                            last_finish_flg =
                                None;


                            pending_finish_video_check =
                                0;


                            finish_battle_address =
                                None;
                        }


                        // ==================================================
                        // stageNum
                        // ==================================================

                        if last_stage_num
                            != Some(stage_num)
                        {
                            asr::print_message(
                                &format!(
                                    "stageNum = {}",
                                    stage_num
                                )
                            );


                            last_stage_num =
                                Some(stage_num);
                        }


                        // ==================================================
                        // startFlg
                        //
                        // タイマー条件には使わない。
                        // 動作確認ログとして保持。
                        // ==================================================

                        if last_start_flg
                            != Some(start_flg)
                        {
                            asr::print_message(
                                &format!(
                                    "startFlg = {}",
                                    start_flg
                                )
                            );


                            last_start_flg =
                                Some(start_flg);
                        }


                        // ==================================================
                        // 3. RESET + START
                        //
                        // 条件:
                        //
                        // ・別Sceneから入場
                        // ・stageNum == 0
                        //
                        // つまりStage Select → Stage 1。
                        //
                        // Stage1 RetryはScene indexが変わらないので
                        // ここには入らない。
                        // ==================================================

                        if scene_transition_pending {
                            if stage_num == 0 {
                                // ------------------------------------------
                                // 実際のLiveSplit操作
                                // ------------------------------------------

                                timer::reset();

                                timer::start();

                                timer::set_game_time(
                                    Duration::milliseconds(
                                        START_TIME_CORRECTION_MS
                                    )
                                );


                                asr::print_message(
                                    "TIMER: RESET + START"
                                );


                                asr::print_message(
                                    &format!(
                                        "Game Time correction = +{} ms",
                                        START_TIME_CORRECTION_MS
                                    )
                                );
                            }


                            // Stage 1でも他Stageでも、
                            // 今回のScene遷移についての判定は終了。
                            scene_transition_pending =
                                false;
                        }


                        // ==================================================
                        // 4. finishFlg Edge
                        // ==================================================

                        let finish_edge =
                            last_finish_flg
                                == Some(0)

                            &&

                            finish_flg
                                == 1;


                        if last_finish_flg
                            != Some(finish_flg)
                        {
                            asr::print_message(
                                &format!(
                                    "finishFlg = {}",
                                    finish_flg
                                )
                            );


                            last_finish_flg =
                                Some(finish_flg);
                        }


                        // ==================================================
                        // 5. Victory / Game Over
                        //
                        // finishFlg 0→1:
                        //
                        // video native alive
                        //     → VICTORY
                        //     → SPLIT
                        //
                        // video null/destroyed
                        //     → GAME OVER
                        //     → NO SPLIT
                        // ==================================================

                        if finish_edge {
                            match unity_reference_alive(
                                &process,
                                battle_address,
                                video_offset,
                                pointer_size,
                            ) {
                                // ------------------------------------------
                                // Victory
                                // ------------------------------------------

                                Some(true) => {
                                    timer::split();


                                    asr::print_message(
                                        "RESULT: VICTORY -> SPLIT"
                                    );
                                }


                                // ------------------------------------------
                                // Game Over
                                // ------------------------------------------

                                Some(false) => {
                                    asr::print_message(
                                        "RESULT: GAME OVER -> NO SPLIT"
                                    );
                                }


                                // ------------------------------------------
                                // Rare read failure
                                // ------------------------------------------

                                None => {
                                    pending_finish_video_check =
                                        FINISH_VIDEO_RETRY_TICKS;


                                    finish_battle_address =
                                        Some(
                                            battle_address
                                        );


                                    asr::print_message(
                                        "video read failed; retrying..."
                                    );
                                }
                            }
                        }
                    }


                    // ======================================================
                    // 6. Video Read Fallback
                    //
                    // RTSelect待ちではない。
                    //
                    // finish tickだけvideoのメモリreadに失敗した場合だけ
                    // 最大30tick再試行。
                    // ======================================================

                    if pending_finish_video_check > 0 {
                        if let Some(
                            battle_address
                        ) =
                            finish_battle_address
                        {
                            match unity_reference_alive(
                                &process,
                                battle_address,
                                video_offset,
                                pointer_size,
                            ) {
                                // ------------------------------------------
                                // Victory
                                // ------------------------------------------

                                Some(true) => {
                                    timer::split();


                                    asr::print_message(
                                        "RESULT: VICTORY -> SPLIT (retry)"
                                    );


                                    pending_finish_video_check =
                                        0;


                                    finish_battle_address =
                                        None;
                                }


                                // ------------------------------------------
                                // Game Over
                                // ------------------------------------------

                                Some(false) => {
                                    asr::print_message(
                                        "RESULT: GAME OVER -> NO SPLIT (retry)"
                                    );


                                    pending_finish_video_check =
                                        0;


                                    finish_battle_address =
                                        None;
                                }


                                // ------------------------------------------
                                // Still unavailable
                                // ------------------------------------------

                                None => {
                                    pending_finish_video_check -=
                                        1;


                                    if pending_finish_video_check
                                        == 0
                                    {
                                        asr::print_message(
                                            "ERROR: Could not determine finish result."
                                        );


                                        finish_battle_address =
                                            None;
                                    }
                                }
                            }
                        }

                        else {
                            pending_finish_video_check =
                                0;
                        }
                    }


                    next_tick().await;
                }
            })
            .await;


        asr::print_message(
            "ScreensaverBATTLE.exe closed. Waiting for restart..."
        );
    }
}