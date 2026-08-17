# ScreenSaver BATTLE Auto Splitter

Steam 版の ScreenSaver BATTLE 用 LiveSplit Auto Splitter です。

このプロジェクトは Rust と LiveSplit ASR を使い、WASM 形式の Auto Splitting Runtime を生成します。

現時点では Steam 版を対象としています。Switch 版対応については記載していません。

---

## 日本語

### 概要

この Auto Splitter は、ゲーム内の Unity Mono オブジェクトを監視して、Stage 1 の開始と各ステージの終了を LiveSplit に通知します。

利用者が追加するのは、リポジトリのビルド成果物ではなく、GitHub Releases で配布される完成済みの `.wasm` ファイルです。

この README だけを見れば、一般利用者が LiveSplit に導入して使い始める流れが確認できます。

### 機能

- Stage 1 に入った時に Run を Reset して Start する
- 各ステージで VICTORY 時に自動で Split する
- GAME OVER 時には Split しない
- Stage 内の Retry ではタイマーを Reset しない
- Stage Select へ戻って Stage 1 を再選択した場合は、新しい Run として再スタートする
- LiveSplit の Timing Method は Game Time を前提とする

### 動作仕様

#### 自動スタート / リセット

別のシーンから Stage 1 へ入った際に、

- 現在の LiveSplit の Run を Reset
- 新しい Run を Start

します。

つまり、途中で記録が悪くなって Stage Select に戻り、再び Stage 1 を選んだ場合でも、自動的にリセットして新しい計測を開始します。

ゲーム自体を再起動する必要はありません。

ただし、Stage 1 をプレイ中にゲーム内の Retry を行った場合は、タイマーを Reset / Restart しません。

#### 自動 Split

各ステージで「VICTORY」になった際に自動で Split します。

「GAME OVER」では Split しません。

最終ステージも、特殊な Stop 処理ではなく、通常の LiveSplit による最後の Split として扱います。

#### Timing Method

この Auto Splitter を利用する場合、LiveSplit の Timing Method は必ず Game Time を使用してください。

これは重要です。Auto Splitter はゲーム中の時間軸に対して動作しており、実ゲームの開始状態と連動するためです。

#### +1.000 秒の補正について

ゲームでは Stage Select でステージを決定してから、約 1 秒の Fade 処理を経て Battle Scene がロードされます。

この Auto Splitter は Battle Scene 側で Stage 1 への入場を検出しているため、計測開始時に Game Time に +1.000 秒を設定して、この Fade 時間を補正しています。

この補正は実測で確認したものです。

そのため、タイマーが 0 秒ではなく約 1 秒から始まるように見えるのは仕様です。これは「1 秒遅れている不具合」ではなく、Stage Select の演出時間を考慮した補正です。

### 導入方法

通常の利用者は Rust や Cargo をインストールして自分でビルドする必要はありません。

GitHub Releases で配布される完成済みの `.wasm` ファイルを使う想定です。

1. この GitHub リポジトリの Releases を開く
2. 最新の Release から `.wasm` ファイルをダウンロードする
3. LiveSplit を起動する
4. LiveSplit を右クリックして Edit Layout を開く
5. 「+」から Control → Auto Splitting Runtime を追加する
6. Layout Settings から Auto Splitting Runtime の設定を開く
7. ダウンロードした `.wasm` ファイルを指定する
8. LiveSplit の Timing Method を Game Time に設定する
9. ScreenSaver BATTLE を起動する
10. Stage 1 を選択し、自動で Reset / Start されることを確認する

このリポジトリで実際に生成される WASM は、Cargo の crate 名と出力設定に基づいて次のファイル名になります。

```text
target/wasm32-unknown-unknown/release/screensaver_battle_autosplitter.wasm
```

GitHub Releases の Asset として配布するのが想定です。`target/` 配下のビルド成果物をリポジトリ自体に含める運用は行いません。

### LiveSplit 側の Split 設定

Auto Splitter 自体は VICTORY ごとに Split を送信するため、LiveSplit 側では実際に走るルートに合わせた Segment を用意する必要があります。

たとえば、次のような形を作るイメージです。

- Stage 1
- Stage 2
- Stage 3
- ...

Segment の数や構成は、実際のプレイルートと分割の考え方に合わせて調整してください。

このプロジェクトがソース上で明確に全ステージ数を定義しているわけではないため、勝手に全ステージを断定することはしません。

### 動作確認

導入後は次を確認してください。

- Stage 1 に入ると Reset + Start される
- VICTORY で Split される
- GAME OVER では Split されない
- ステージ内 Retry でタイマーが Reset されない
- Stage Select に戻って Stage 1 を再選択すると、新しい Run として Reset + Start される

### トラブルシューティング

#### Auto Splitter が動かない

確認項目は次のとおりです。

- Steam 版の ScreenSaver BATTLE を使用しているか
- Auto Splitting Runtime に正しい `.wasm` を指定しているか
- ゲームが起動しているか
- LiveSplit の Timing Method が Game Time になっているか

#### タイマーが 0 秒ではなく約 1 秒から始まる

これは仕様です。

Stage Select の Fade 時間を補正するため、Stage 1 検出時に Game Time に +1.000 秒を設定しています。

#### Retry したのにタイマーがリセットされない

これも仕様です。

Stage 内の Retry は同じ Run の継続として扱います。Stage Select に戻り、Stage 1 を改めて開始した場合にのみ、Reset + Start します。

#### ゲーム更新後に動かなくなった

この Auto Splitter はゲームのメモリを読み取って動作しているため、ゲーム更新後に動作しなくなる可能性があります。

問題が発生した場合は、リポジトリの Issues が有効であればそちらで報告してください。Issue の有無はリポジトリ側で確認できる場合に案内します。

### 自分でビルドする場合

一般利用者にはこの作業は不要です。これは開発者向けの情報です。

このリポジトリでは、Rust の安定版が `rust-toolchain` に設定されており、`.cargo/config.toml` で `wasm32-unknown-unknown` をターゲットにしています。

必要な準備:

```bash
rustup target add wasm32-unknown-unknown --toolchain stable
```

Release ビルド:

```bash
cargo build --release
```

または短縮形:

```bash
cargo b --release
```

生成される WASM の出力先は次のとおりです。

```text
target/wasm32-unknown-unknown/release/screensaver_battle_autosplitter.wasm
```

このプロジェクトは `cdylib` を生成する Rust プロジェクトであり、LiveSplit の Auto Splitting Runtime に読み込ませるのはこの WASM です。

### 技術メモ

この Auto Splitter の基本的な仕組みは、次の通りです。

- Unity Mono の `BattleManager` を監視する
- Scene の変化と `stageNum` から Stage 1 への入場を判断する
- `finishFlg` の変化からステージ終了を判断する
- `BattleManager.video` の Unity native object の状態から VICTORY / GAME OVER を判定する
- `BattleManager` の複数の pointer path に対応する
- StageSelectManager には依存しない

具体的なメモリアドレスや pointer の値は、利用者向け README には記載しません。これは開発用途の補助情報としてのみ扱うべきためです。

### 開発協力

Auto Splitterの調査・設計・実装にあたり、以下の支援を利用しました。

- OpenAI ChatGPT (GPT-5.6 Sol)


### ライセンス

このプロジェクトは [MIT License](LICENSE) の下で公開されています。

Copyright (c) 2026 usuk-bell (usuk_bell)

---

## English

### Overview

This project is a LiveSplit Auto Splitter for the Steam version of ScreenSaver BATTLE.

It is written in Rust and uses LiveSplit ASR to generate a WASM-based Auto Splitting Runtime.

This repository is intended for the Steam version only. It does not claim support for the Switch version.

### Features

- Reset and start a new run when entering Stage 1 from another scene
- Auto split when each stage reaches VICTORY
- Do not split on GAME OVER
- Do not reset the timer on Retry inside a stage
- Reset and restart when returning to Stage Select and selecting Stage 1 again
- Requires LiveSplit Timing Method to be set to Game Time

### Behavior

#### Auto start / reset

When the game transitions from another scene into Stage 1, the splitter will:

- Reset the current LiveSplit run
- Start a new run

This also applies when a run becomes unstable and the player returns to Stage Select, then chooses Stage 1 again.

No game restart is required.

If the player retries inside Stage 1, the timer is not reset or restarted.

#### Auto split

The splitter triggers a split when a stage reaches VICTORY.

It does not split on GAME OVER.

The final stage is handled as a normal final split rather than a special stop condition.

#### Timing Method

When using this Auto Splitter, LiveSplit must use Game Time as the Timing Method.

This is required because the splitter is designed around the in-game time flow and the stage start detection.

#### About the +1.000 second correction

In the game, after a stage is selected in Stage Select, there is a fade/loading period of roughly 1 second before the Battle Scene fully loads.

This splitter detects the Stage 1 entrance in the Battle Scene, so it sets the Game Time to +1.000 seconds at the start of the run to compensate for that fade time.

This is intentional and confirmed by testing.

Therefore, a timer appearing to begin at about 1 second is expected behavior and is not a bug.

### Installation

Most users do not need to install Rust or build the project themselves.

The intended workflow is to download the finished `.wasm` file from the repository's GitHub Releases.

1. Open the Releases page for this repository
2. Download the latest `.wasm` asset
3. Launch LiveSplit
4. Right-click LiveSplit and open Edit Layout
5. Add Control → Auto Splitting Runtime from the layout menu
6. Open the Auto Splitting Runtime settings in Layout Settings
7. Select the downloaded `.wasm` file
8. Set LiveSplit Timing Method to Game Time
9. Start ScreenSaver BATTLE
10. Select Stage 1 and confirm that the run resets and starts automatically

The generated WASM file in this repository is:

```text
target/wasm32-unknown-unknown/release/screensaver_battle_autosplitter.wasm
```

This is the asset intended for release through GitHub Releases, not something to keep in the repository itself.

### LiveSplit split setup

Because the splitter sends a split at each VICTORY, the LiveSplit layout should contain segments matching the actual route you are running.

For example:

- Stage 1
- Stage 2
- Stage 3
- ...

The exact number of segments depends on your route and split structure, and this repository does not define a fixed total stage count for all runs.

### Troubleshooting

#### Auto Splitter does not work

Check the following:

- You are using the Steam version of ScreenSaver BATTLE
- The correct `.wasm` file is selected in Auto Splitting Runtime
- The game is running
- LiveSplit Timing Method is set to Game Time

#### The timer starts at about 1 second instead of 0

This is expected behavior.

The splitter adds +1.000 seconds when Stage 1 is detected to compensate for the Stage Select fade/loading delay.

#### A retry happened but the timer was not reset

This is also intentional.

Retries inside a stage are treated as part of the same run. A reset happens only when the player returns to Stage Select and starts Stage 1 again from there.

#### The splitter stopped working after a game update

This Auto Splitter reads game memory directly, so gameplay updates may break detection in the future.

If this happens, please report it through the repository's Issues page if that feature is enabled.

### Building from source

This work is not required for most users. The following is for developers.

This repository uses the stable Rust toolchain from `rust-toolchain`, and `.cargo/config.toml` targets `wasm32-unknown-unknown`.

Install the target:

```bash
rustup target add wasm32-unknown-unknown --toolchain stable
```

Release build:

```bash
cargo build --release
```

or:

```bash
cargo b --release
```

The output file is:

```text
target/wasm32-unknown-unknown/release/screensaver_battle_autosplitter.wasm
```

### Technical notes

The basic implementation is structured as follows:

- Monitor the Unity Mono `BattleManager`
- Detect entering Stage 1 from scene changes and `stageNum`
- Detect a stage finish from `finishFlg`
- Determine VICTORY / GAME OVER from the state of `BattleManager.video`
- Support multiple BattleManager pointer paths
- Do not depend on StageSelectManager

The exact pointer addresses are intentionally not included in the user-facing README.

### Acknowledgements

Development assistance was provided using:

- OpenAI ChatGPT (GPT-5.6 Sol)

### License

This project is licensed under the [MIT License](LICENSE).

Copyright (c) 2026 usuk-bell (usuk_bell)
