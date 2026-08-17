#![no_std]

extern crate alloc;

use alloc::format;
use asr::{
    future::next_tick,
    game_engine::unity::{
        mono::{Class, Module},
        scene_manager::SceneManager,
    },
    timer,
    Address,
    PointerSize,
    Process,
};
use time::Duration;

#[global_allocator]
static ALLOC: dlmalloc::GlobalDlmalloc = dlmalloc::GlobalDlmalloc;

asr::async_main!(stable);
asr::panic_handler!();

const VERSION: &str = "1.0.0";

// Stage SelectでStage 1を決定してからBattleManagerが取得できるまでの補正。
// 実測・耐久テスト済み。
const START_TIME_CORRECTION_MS: i64 = 1000;

// finishFlg成立時にvideoのreadだけ失敗した場合の短時間再試行。
const FINISH_VIDEO_RETRY_TICKS: u8 = 30;

// Unity 64-bit Mono: UnityEngine.Object.m_CachedPtr
const UNITY_OBJECT_CACHED_PTR: u64 = 0x10;

// -------------------------------------------------------------------------
// BattleManager pointer families
//
// Family A:
// mono-2.0-bdwgc.dll + 0x7390F8
//   -> +0x60
//   -> +0x600 / +0x540
//
// Family B:
// mono-2.0-bdwgc.dll + 0x763240
//   -> +0x260
//   -> +0x600 / +0x540
//
// Retryによって0x600 / 0x540配置が切り替わるため両方を監視する。
// -------------------------------------------------------------------------

const BATTLE_A_BASE: u64 = 0x007390F8;
const BATTLE_A_600_OFFSETS: &[u64] = &[0x60, 0x600];
const BATTLE_A_540_OFFSETS: &[u64] = &[0x60, 0x540];

const BATTLE_B_BASE: u64 = 0x00763240;
const BATTLE_B_600_OFFSETS: &[u64] = &[0x260, 0x600];
const BATTLE_B_540_OFFSETS: &[u64] = &[0x260, 0x540];

fn read_non_null_pointer(
    process: &Process,
    address: Address,
    pointer_size: PointerSize,
) -> Option<Address> {
    process
        .read_pointer(address, pointer_size)
        .ok()
        .filter(|address| !address.is_null())
}

fn resolve_ce_pointer(
    process: &Process,
    base: Address,
    offsets: &[u64],
    pointer_size: PointerSize,
) -> Option<Address> {
    let (&last_offset, parent_offsets) = offsets.split_last()?;

    let mut address =
        read_non_null_pointer(process, base, pointer_size)?;

    for &offset in parent_offsets {
        address =
            read_non_null_pointer(
                process,
                address + offset,
                pointer_size,
            )?;
    }

    Some(address + last_offset)
}

fn unity_native_alive(
    process: &Process,
    managed_object: Address,
    pointer_size: PointerSize,
) -> Option<bool> {
    process
        .read_pointer(
            managed_object + UNITY_OBJECT_CACHED_PTR,
            pointer_size,
        )
        .ok()
        .map(|native| !native.is_null())
}

fn unity_reference_alive(
    process: &Process,
    owner: Address,
    field_offset: u32,
    pointer_size: PointerSize,
) -> Option<bool> {
    let managed_reference = process
        .read_pointer(
            owner + field_offset as u64,
            pointer_size,
        )
        .ok()?;

    if managed_reference.is_null() {
        return Some(false);
    }

    unity_native_alive(
        process,
        managed_reference,
        pointer_size,
    )
}

async fn get_field_offset(
    process: &Process,
    mono: &Module,
    class: &Class,
    field_name: &str,
) -> u32 {
    class
        .wait_get_field_offset(
            process,
            mono,
            field_name,
        )
        .await
}

fn read_battle_values(
    process: &Process,
    address: Address,
    stage_num_offset: u32,
    start_flg_offset: u32,
    finish_flg_offset: u32,
    pointer_size: PointerSize,
) -> Option<(i32, u8, u8)> {
    if unity_native_alive(
        process,
        address,
        pointer_size,
    ) == Some(false)
    {
        return None;
    }

    let stage_num = process
        .read::<i32>(
            address + stage_num_offset as u64
        )
        .ok()?;

    let start_flg = process
        .read::<u8>(
            address + start_flg_offset as u64
        )
        .ok()?;

    let finish_flg = process
        .read::<u8>(
            address + finish_flg_offset as u64
        )
        .ok()?;

    if !(0..=20).contains(&stage_num)
        || start_flg > 1
        || finish_flg > 1
    {
        return None;
    }

    Some((
        stage_num,
        start_flg,
        finish_flg,
    ))
}

fn try_battle_candidate(
    process: &Process,
    mono_base: Address,
    base_offset: u64,
    offsets: &[u64],
    source: u8,
    stage_num_offset: u32,
    start_flg_offset: u32,
    finish_flg_offset: u32,
    pointer_size: PointerSize,
) -> Option<(Address, u8, i32, u8, u8)> {
    let address = resolve_ce_pointer(
        process,
        mono_base + base_offset,
        offsets,
        pointer_size,
    )?;

    let (
        stage_num,
        start_flg,
        finish_flg,
    ) = read_battle_values(
        process,
        address,
        stage_num_offset,
        start_flg_offset,
        finish_flg_offset,
        pointer_size,
    )?;

    Some((
        address,
        source,
        stage_num,
        start_flg,
        finish_flg,
    ))
}

fn resolve_active_battle_manager(
    process: &Process,
    mono_base: Address,
    preferred: Option<Address>,
    stage_num_offset: u32,
    start_flg_offset: u32,
    finish_flg_offset: u32,
    pointer_size: PointerSize,
) -> Option<(Address, u8, i32, u8, u8)> {
    // 現在のinstanceがまだ生存していれば優先する。
    if let Some(address) = preferred {
        if let Some((
            stage_num,
            start_flg,
            finish_flg,
        )) = read_battle_values(
            process,
            address,
            stage_num_offset,
            start_flg_offset,
            finish_flg_offset,
            pointer_size,
        ) {
            return Some((
                address,
                0,
                stage_num,
                start_flg,
                finish_flg,
            ));
        }
    }

    try_battle_candidate(
        process,
        mono_base,
        BATTLE_A_BASE,
        BATTLE_A_600_OFFSETS,
        1,
        stage_num_offset,
        start_flg_offset,
        finish_flg_offset,
        pointer_size,
    )
    .or_else(|| {
        try_battle_candidate(
            process,
            mono_base,
            BATTLE_A_BASE,
            BATTLE_A_540_OFFSETS,
            2,
            stage_num_offset,
            start_flg_offset,
            finish_flg_offset,
            pointer_size,
        )
    })
    .or_else(|| {
        try_battle_candidate(
            process,
            mono_base,
            BATTLE_B_BASE,
            BATTLE_B_600_OFFSETS,
            3,
            stage_num_offset,
            start_flg_offset,
            finish_flg_offset,
            pointer_size,
        )
    })
    .or_else(|| {
        try_battle_candidate(
            process,
            mono_base,
            BATTLE_B_BASE,
            BATTLE_B_540_OFFSETS,
            4,
            stage_num_offset,
            start_flg_offset,
            finish_flg_offset,
            pointer_size,
        )
    })
}

async fn main() {
    asr::print_message(
        &format!(
            "ScreenSaver BATTLE Auto Splitter v{}",
            VERSION
        )
    );

    loop {
        let process =
            Process::wait_attach(
                "ScreensaverBATTLE.exe"
            )
            .await;

        process
            .until_closes(async {
                let mono =
                    Module::wait_attach_auto_detect(
                        &process
                    )
                    .await;

                let pointer_size =
                    mono.get_pointer_size();

                if !matches!(
                    pointer_size,
                    PointerSize::Bit64
                ) {
                    asr::print_message(
                        "Unsupported process architecture."
                    );

                    loop {
                        next_tick().await;
                    }
                }

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

                let stage_num_offset =
                    get_field_offset(
                        &process,
                        &mono,
                        &battle_class,
                        "stageNum",
                    )
                    .await;

                let start_flg_offset =
                    get_field_offset(
                        &process,
                        &mono,
                        &battle_class,
                        "startFlg",
                    )
                    .await;

                let finish_flg_offset =
                    get_field_offset(
                        &process,
                        &mono,
                        &battle_class,
                        "finishFlg",
                    )
                    .await;

                let video_offset =
                    get_field_offset(
                        &process,
                        &mono,
                        &battle_class,
                        "video",
                    )
                    .await;

                let scene_manager =
                    SceneManager::wait_attach(
                        &process
                    )
                    .await;

                let mono_base = loop {
                    match process
                        .get_module_address(
                            "mono-2.0-bdwgc.dll"
                        )
                    {
                        Ok(address) =>
                            break address,

                        Err(_) =>
                            next_tick().await,
                    }
                };

                asr::print_message(
                    "Auto Splitter ready."
                );

                let mut last_scene_index:
                    Option<i32> = None;

                let mut scene_transition_pending =
                    false;

                let mut current_battle:
                    Option<Address> = None;

                let mut last_stage_num:
                    Option<i32> = None;

                let mut last_start_flg:
                    Option<u8> = None;

                let mut last_finish_flg:
                    Option<u8> = None;

                let mut pending_finish_video_check:
                    u8 = 0;

                let mut finish_battle_address:
                    Option<Address> = None;

                loop {
                    // ------------------------------------------------------
                    // Scene transition detection
                    // ------------------------------------------------------

                    if let Ok(scene_index) =
                        scene_manager
                            .get_current_scene_index(
                                &process
                            )
                    {
                        if scene_index >= 0 {
                            match last_scene_index {
                                None => {
                                    last_scene_index =
                                        Some(scene_index);
                                }

                                Some(previous)
                                    if previous
                                        != scene_index =>
                                {
                                    last_scene_index =
                                        Some(scene_index);

                                    scene_transition_pending =
                                        true;

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

                    // ------------------------------------------------------
                    // BattleManager
                    // ------------------------------------------------------

                    if let Some((
                        battle_address,
                        source,
                        stage_num,
                        start_flg,
                        finish_flg,
                    )) = resolve_active_battle_manager(
                        &process,
                        mono_base,
                        current_battle,
                        stage_num_offset,
                        start_flg_offset,
                        finish_flg_offset,
                        pointer_size,
                    ) {
                        let battle_changed =
                            match current_battle {
                                Some(old) =>
                                    old.value()
                                        != battle_address.value(),

                                None =>
                                    true,
                            };

                        if battle_changed {
                            current_battle =
                                Some(battle_address);

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

                            let source_name =
                                match source {
                                    1 => "A600",
                                    2 => "A540",
                                    3 => "B600",
                                    4 => "B540",
                                    _ => "cached",
                                };

                            asr::print_message(
                                &format!(
                                    "BattleManager updated ({})",
                                    source_name
                                )
                            );
                        }

                        last_stage_num =
                            Some(stage_num);

                        last_start_flg =
                            Some(start_flg);

                        // --------------------------------------------------
                        // Stage 1 entry -> reset + start
                        // --------------------------------------------------

                        if scene_transition_pending {
                            if stage_num == 0 {
                                timer::reset();
                                timer::start();

                                timer::set_game_time(
                                    Duration::milliseconds(
                                        START_TIME_CORRECTION_MS
                                    )
                                );

                                asr::print_message(
                                    "Timer reset + started."
                                );
                            }

                            scene_transition_pending =
                                false;
                        }

                        // --------------------------------------------------
                        // finishFlg 0 -> 1
                        // --------------------------------------------------

                        let finish_edge =
                            last_finish_flg
                                == Some(0)
                            && finish_flg == 1;

                        last_finish_flg =
                            Some(finish_flg);

                        if finish_edge {
                            match unity_reference_alive(
                                &process,
                                battle_address,
                                video_offset,
                                pointer_size,
                            ) {
                                Some(true) => {
                                    timer::split();

                                    asr::print_message(
                                        "Victory -> split."
                                    );
                                }

                                Some(false) => {
                                    asr::print_message(
                                        "Game Over -> no split."
                                    );
                                }

                                None => {
                                    pending_finish_video_check =
                                        FINISH_VIDEO_RETRY_TICKS;

                                    finish_battle_address =
                                        Some(battle_address);
                                }
                            }
                        }
                    }

                    // ------------------------------------------------------
                    // Rare video-read retry
                    // ------------------------------------------------------

                    if pending_finish_video_check > 0 {
                        if let Some(battle_address) =
                            finish_battle_address
                        {
                            match unity_reference_alive(
                                &process,
                                battle_address,
                                video_offset,
                                pointer_size,
                            ) {
                                Some(true) => {
                                    timer::split();

                                    pending_finish_video_check =
                                        0;

                                    finish_battle_address =
                                        None;

                                    asr::print_message(
                                        "Victory -> split."
                                    );
                                }

                                Some(false) => {
                                    pending_finish_video_check =
                                        0;

                                    finish_battle_address =
                                        None;

                                    asr::print_message(
                                        "Game Over -> no split."
                                    );
                                }

                                None => {
                                    pending_finish_video_check -=
                                        1;

                                    if pending_finish_video_check
                                        == 0
                                    {
                                        finish_battle_address =
                                            None;

                                        asr::print_message(
                                            "Unable to determine finish result."
                                        );
                                    }
                                }
                            }
                        } else {
                            pending_finish_video_check =
                                0;
                        }
                    }

                    next_tick().await;
                }
            })
            .await;
    }
}