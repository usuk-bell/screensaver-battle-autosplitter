# ScreenSaver BATTLE Auto Splitter

Steam 版の **ScreenSaver BATTLE** 用 LiveSplit Auto Splitter です。

このプロジェクトは Rust と LiveSplit ASR を使用し、WASM 形式の Auto Splitter を生成します。

現時点では Steam 版のみを対象としています。Switch 版には対応していません。

---

## 日本語

### 概要

この Auto Splitter は、ゲーム内の Unity Mono / Hierarchy 情報から `BattleManager` を取得し、Stage 1 の開始と各ステージの終了を LiveSplit に通知します。

一般利用者は Rust や Cargo をインストールする必要はありません。GitHub Releases で配布される完成済みの `.wasm` ファイルを使用してください。

### 機能

- Stage 1 に入った時に Run を Reset して Start する
- 各ステージで VICTORY 時に自動で Split する
- GAME OVER 時には Split しない
- Stage 内の Retry ではタイマーを Reset しない
- Stage Select へ戻って Stage 1 を再選択した場合は、新しい Run として Reset + Start する
- LiveSplit の Timing Method は Game Time を使用する

### 動作仕様

#### 自動スタート / リセット

別のシーンから Stage 1 へ入った際に、現在の LiveSplit の Run を Reset し、新しい Run を Start します。

途中で Stage Select に戻り、再び Stage 1 を選んだ場合も、自動的に Reset + Start します。ゲーム自体を再起動する必要はありません。

Stage 1 を含め、ステージ内でゲームの Retry を行った場合はタイマーを Reset / Restart しません。

#### 自動 Split

各ステージで「VICTORY」になった際に自動で Split します。

「GAME OVER」では Split しません。

最終ステージも特殊な Stop 処理ではなく、通常の LiveSplit による最後の Split として扱います。

#### Timing Method

この Auto Splitter を利用する場合、LiveSplit の Timing Method は **Game Time** に設定してください。

#### +1.000 秒の補正について

ゲームでは Stage Select でステージを決定してから、約 1 秒の Fade 処理を経て Battle Scene がロードされます。

この Auto Splitter は Battle Scene 側で Stage 1 への入場を検出しているため、計測開始時に Game Time を `+1.000` 秒に設定し、Stage Select 側の Fade 時間を補正しています。

そのため、タイマーが 0 秒ではなく約 1 秒から始まるように見えるのは仕様です。

### 導入方法

1. この GitHub リポジトリの Releases を開く
2. 最新の Release から `.wasm` ファイルをダウンロードする
3. LiveSplit を起動する
4. LiveSplit を右クリックして **Edit Layout** を開く
5. `+` から **Control → Auto Splitting Runtime** を追加する
6. **Layout Settings** から Auto Splitting Runtime の設定を開く
7. ダウンロードした `.wasm` ファイルを指定する
8. LiveSplit の Timing Method を **Game Time** に設定する
9. ScreenSaver BATTLE を起動する
10. Stage 1 を選択し、自動で Reset / Start されることを確認する

このリポジトリで生成される Release 用 WASM は次のファイルです。

```text
target/wasm32-unknown-unknown/release/screensaver_battle_autosplitter.wasm
```

この `.wasm` を GitHub Releases の Asset として配布します。`target/` 配下のビルド成果物はリポジトリ自体には含めません。

### LiveSplit 側の Split 設定

Auto Splitter は VICTORY ごとに Split を送信するため、LiveSplit 側では実際に走るルートに合わせた Segment を用意してください。

例:

- Stage 1
- Stage 2
- Stage 3
- ...

Segment の数や構成は、実際のプレイルートと分割方法に合わせて調整してください。

### 動作確認

現在の実装では、開発環境に加えて別PC環境でも以下の動作を確認しています。

- Stage 1 入場時の Reset + Start
- VICTORY 時の Split
- GAME OVER 時に Split しない
- ステージ内 Retry でタイマーを Reset しない
- Retry によって `BattleManager` が再生成された場合も追従する
- Stage Select に戻って Stage 1 を再選択した場合の Reset + Start

### トラブルシューティング

#### Auto Splitter が動かない

次を確認してください。

- Steam 版の ScreenSaver BATTLE を使用しているか
- Auto Splitting Runtime に最新の `.wasm` を指定しているか
- ゲームが起動しているか
- LiveSplit の Timing Method が Game Time になっているか

#### タイマーが 0 秒ではなく約 1 秒から始まる

仕様です。

Stage Select の Fade 時間を補正するため、Stage 1 検出時に Game Time を `+1.000` 秒に設定しています。

#### Retry したのにタイマーがリセットされない

仕様です。

Stage 内の Retry は同じ Run の継続として扱います。Stage Select に戻り、Stage 1 を改めて開始した場合にのみ Reset + Start します。

#### ゲーム更新後に動かなくなった

この Auto Splitter はゲームのメモリおよび Unity の内部構造を読み取って動作しているため、ゲームやUnityバージョンの更新によって動作しなくなる可能性があります。

問題が発生した場合は、GitHub Issues から報告してください。

### 自分でビルドする場合

一般利用者には不要です。以下は開発者向けの情報です。

このリポジトリでは、Rust の安定版を使用し、`.cargo/config.toml` で `wasm32-unknown-unknown` をターゲットにしています。

ターゲットの追加:

```bash
rustup target add wasm32-unknown-unknown --toolchain stable
```

Release ビルド:

```bash
cargo build --release
```

生成される WASM:

```text
target/wasm32-unknown-unknown/release/screensaver_battle_autosplitter.wasm
```

このプロジェクトは `cdylib` を生成する Rust プロジェクトであり、LiveSplit の Auto Splitting Runtime に読み込ませるのはこの WASM です。

### 技術メモ

現在の実装は次のような構成です。

- ASR の Unity Mono 機能から `Assembly-CSharp` と `BattleManager` の型情報を取得する
- `stageNum`、`startFlg`、`finishFlg`、`video` のフィールドオフセットを Mono metadata から名前で取得する
- Scene の変化と `stageNum` から Stage 1 への入場を判断する
- Unity 2023.2.20f1 の native Hierarchy を辿り、`BattleManager` GameObject から現在の managed `BattleManager` を取得する
- `finishFlg` の変化からステージ終了を判断する
- `BattleManager.video` の Unity native object の状態から VICTORY / GAME OVER を判定する
- ステージ内 Retry などで `BattleManager` が再生成された場合も、Hierarchy から現在のインスタンスを再取得する
- `mono-2.0-bdwgc.dll` からの固定 pointer path には依存しない
- `StageSelectManager` には依存しない

Unity の内部 offset はゲームの使用する Unity バージョンに依存する実装詳細であり、ゲーム更新後には再調査が必要になる可能性があります。

### 開発協力

Auto Splitter の調査・設計・実装にあたり、以下の支援を利用しました。

- OpenAI ChatGPT (GPT-5.6 Sol)

### ライセンス

このプロジェクトは [MIT License](LICENSE) の下で公開されています。

Copyright (c) 2026 usuk-bell (usuk_bell)

---

## English

### Overview

This project is a LiveSplit Auto Splitter for the Steam version of **ScreenSaver BATTLE**.

It is written in Rust and uses LiveSplit ASR to generate a WASM-based Auto Splitter.

The Steam version is currently supported. The Switch version is not supported.

The splitter locates the game's `BattleManager` through Unity Mono metadata and the Unity scene hierarchy, then uses it to detect the beginning of Stage 1 and the end of each stage.

Most users do not need Rust or Cargo. Download the finished `.wasm` file from GitHub Releases.

### Features

- Reset and start a new run when entering Stage 1 from another scene
- Auto split when each stage reaches VICTORY
- Do not split on GAME OVER
- Do not reset the timer on Retry inside a stage
- Reset and restart when returning to Stage Select and selecting Stage 1 again
- Uses LiveSplit Game Time

### Behavior

#### Auto start / reset

When the game transitions from another scene into Stage 1, the splitter resets the current LiveSplit run and starts a new run.

Returning to Stage Select and selecting Stage 1 again also starts a new run. Restarting the game itself is not required.

Retries inside a stage do not reset or restart the timer.

#### Auto split

The splitter triggers a split when a stage reaches VICTORY.

It does not split on GAME OVER.

The final stage is handled as a normal final split rather than a special stop condition.

#### Timing Method

Set LiveSplit's Timing Method to **Game Time** when using this Auto Splitter.

#### About the +1.000 second correction

After selecting a stage in Stage Select, the game performs roughly one second of fade/loading before the Battle Scene is loaded.

Because the splitter detects the Stage 1 entrance from the Battle Scene, it starts Game Time at `+1.000` seconds to compensate for the Stage Select fade.

A timer appearing to begin at about one second is therefore expected behavior.

### Installation

1. Open the Releases page for this repository
2. Download the latest `.wasm` asset
3. Launch LiveSplit
4. Right-click LiveSplit and open **Edit Layout**
5. Add **Control → Auto Splitting Runtime**
6. Open the Auto Splitting Runtime settings from **Layout Settings**
7. Select the downloaded `.wasm` file
8. Set LiveSplit's Timing Method to **Game Time**
9. Start ScreenSaver BATTLE
10. Select Stage 1 and confirm that the run resets and starts automatically

The Release build generates:

```text
target/wasm32-unknown-unknown/release/screensaver_battle_autosplitter.wasm
```

This `.wasm` file is intended to be uploaded as a GitHub Release asset. Files under `target/` are not intended to be committed to the repository.

### LiveSplit split setup

The splitter sends a split at each VICTORY, so prepare LiveSplit segments that match the route you are running.

For example:

- Stage 1
- Stage 2
- Stage 3
- ...

The exact segment count and structure depend on the route.

### Tested behavior

The current implementation has been tested on the development environment and on an additional PC environment.

Confirmed behavior includes:

- Reset + Start when entering Stage 1
- Split on VICTORY
- No split on GAME OVER
- No timer reset on an in-stage Retry
- Correctly reacquiring `BattleManager` when it is recreated by a Retry
- Reset + Start after returning to Stage Select and selecting Stage 1 again

### Troubleshooting

#### Auto Splitter does not work

Check the following:

- You are using the Steam version of ScreenSaver BATTLE
- The latest `.wasm` file is selected in Auto Splitting Runtime
- The game is running
- LiveSplit's Timing Method is set to Game Time

#### The timer starts at about one second instead of zero

This is expected behavior.

The splitter initializes Game Time at `+1.000` seconds to compensate for the Stage Select fade/loading delay.

#### A retry happened but the timer was not reset

This is intentional.

Retries inside a stage are treated as part of the same run. A reset occurs only after returning to Stage Select and entering Stage 1 again.

#### The splitter stopped working after a game update

This Auto Splitter reads game memory and Unity internal structures directly. A game update or Unity version change may therefore break detection.

If this happens, please report it through GitHub Issues.

### Building from source

This section is intended for developers.

Install the target:

```bash
rustup target add wasm32-unknown-unknown --toolchain stable
```

Build the Release WASM:

```bash
cargo build --release
```

Output:

```text
target/wasm32-unknown-unknown/release/screensaver_battle_autosplitter.wasm
```

The project builds a Rust `cdylib`, and this WASM is the file loaded by LiveSplit's Auto Splitting Runtime.

### Technical notes

The current implementation works as follows:

- Use ASR's Unity Mono support to resolve `Assembly-CSharp` and the `BattleManager` type
- Resolve the `stageNum`, `startFlg`, `finishFlg`, and `video` field offsets by name from Mono metadata
- Detect entering Stage 1 from scene changes and `stageNum`
- Traverse the Unity 2023.2.20f1 native scene hierarchy to locate the current `BattleManager` GameObject and managed instance
- Detect stage completion from `finishFlg`
- Determine VICTORY / GAME OVER from the Unity native object state of `BattleManager.video`
- Reacquire the current `BattleManager` when the game recreates it during an in-stage Retry
- Do not depend on fixed pointer paths from `mono-2.0-bdwgc.dll`
- Do not depend on `StageSelectManager`

The Unity internal offsets used by this implementation are version-dependent details and may require further investigation after a game or Unity update.

### Acknowledgements

Development assistance was provided using:

- OpenAI ChatGPT (GPT-5.6 Sol)

### License

This project is licensed under the [MIT License](LICENSE).

Copyright (c) 2026 usuk-bell (usuk_bell)
