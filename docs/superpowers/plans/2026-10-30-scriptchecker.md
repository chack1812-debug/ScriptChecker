# ScriptChecker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Script2Video の台本・`timeline.json`・`full_dialogue.wav` を読み込み、再生に同期して台本の行(と推定文字位置)を強調し、ルビ併記表示と密着レベルメーター、6操作+シークバーを備えるデスクトップツールを作る。

**Architecture:** 純ロジック(台本パーサ、タイムライン、文字時刻推定、トランスポート/レンダリング)をUIから分離して単体テストし、`eframe/egui` のUI層と `cpal` の出力層を薄く被せる。再生位置は `Transport`(フレーム位置)を唯一の正本にし、台本強調・メーター・シークバーが同じ位置を参照する。

**Tech Stack:** Rust 2024、eframe 0.31、cpal 0.15、hound 3.5、serde/serde_json、sha2 0.10、rfd 0.15。

**Spec:** `docs/superpowers/specs/2026-10-30-scriptchecker-design.md`

## Global Constraints

- 作業場所は `D:\UDS\宇摩コードワークス\ScriptChecker\`(独立Cargoプロジェクト、パッケージ名 `scriptchecker`)。スペアナⅡ側は一切変更しない。メーターのコードはパス依存せずコピー引用する。
- Rust edition 2024、`eframe = "0.31"`(スペアナⅡと同スタック)。
- 入力は `第N回.txt`、同フォルダの `full_dialogue.wav`、`timeline/timeline.json`。行の対応は行ID(例 `S01-L001`)。
- 読み指定は `'単語|読み'`(区切りは半角 `|` のみ)。表示は「単語」、読みはルビとして語の上に小さく併記(ON/OFF切替あり)。
- 6操作: 早戻し再生/10秒戻し/停止/通常再生/10秒送り/早送り再生。ボタン列の下に現在位置シークバー。高速再生は4倍(定数 `FAST_RATE = 4.0`)、早戻しは逆方向再生。10秒は `SKIP_MS = 10_000`。
- メーター: 目盛 -60〜0 dBFS、RMS・ピーク・ピークホールド。チャンネルバーは密着(隙間なし)、右端に細く配置。台本表示エリアが最大。
- 文字重み(初期値): かな=1モーラ、漢字=2、英数字=0.5、`、`=1、`。`=2。小書き`ゃゅょぁぃぅぇぉ`は0、`っ`は1、長音`ー`は1。
- 異常系: wav/timeline欠落はパス付きエラー、台本SHA-256不一致・音声長差は警告(再生は許可)、行ID不一致・`missing:true`は警告でクラッシュしない。
- 工程はBeads(prefix `sck`)で管理。issueのcloseは実装者自身で行わない(独立レビュー後)。push・releaseはHuman承認が必要。commitは各タスク末尾。

## Review Focus

1. 台本にあるが `timeline.json` に無い行 / `missing:true` の行: 強調せずスキップ、警告に件数表示。
2. Windows既定のeguiフォントは日本語を持たない: 起動時にシステム日本語フォント(`C:\Windows\Fonts\YuGothM.ttc` → `meiryo.ttc` の順)を読み込み、全文字が豆腐にならないこと。
3. BOM付き/CRLFの台本、先頭 `#` コメント、`@cast` 以降の説明行: 台詞行(`[Sxx-Lyyy] 話者:台詞`)以外を誤って台詞にしない。
4. 先頭(0秒)での早戻し、末尾での早送り/通常再生、総時間を超える/負のシーク: 停止して位置をクランプ。
5. 台詞内の `'` が読み指定でない場合(肉声役、閉じていない `'`): 文字としてそのまま表示し、パーサが落ちない。

---

### Task 0: プロジェクト雛形とBeads issue起票

**Files:**
- Create: `Cargo.toml`, `src/main.rs`, `src/lib.rs`
- Modify: `.gitignore`(`target/` は既存)

**Interfaces:**
- Produces: ライブラリ crate `scriptchecker`(`src/lib.rs` にモジュール宣言を追加していく)。

- [ ] **Step 1:** `bd create` で Task 1〜9 に対応するissueを起票し(`bd create "T1 台本パーサ" -p 1` 等、依存は `bd dep add`)、`bd ready` で T1 が出ることを確認する。
- [ ] **Step 2:** `cargo init --name scriptchecker` 相当で `Cargo.toml`(上記依存を記述、`[lib]` と `[[bin]]` 併設)、`src/lib.rs`(空)、`src/main.rs`(`fn main() {}`)を作る。
- [ ] **Step 3:** `cargo build` が成功することを確認する。
- [ ] **Step 4: Commit** `chore: scaffold ScriptChecker crate`

### Task 1: 台本パーサ(`src/script.rs`)

**Files:** Create `src/script.rs`, Test: 同ファイル内 `#[cfg(test)]`

**Interfaces:**
- Produces:
  ```rust
  pub enum Seg { Plain(String), Ruby { base: String, reading: String } }
  pub struct ScriptLine { pub id: String, pub scene: String, pub cast: String, pub segs: Vec<Seg> }
  pub struct SceneHead { pub id: String, pub name: String }
  pub enum ScriptItem { Scene(SceneHead), Line(ScriptLine) }
  pub fn parse_script(text: &str) -> Vec<ScriptItem>
  pub fn parse_segs(text: &str) -> Vec<Seg>
  pub fn display_text(segs: &[Seg]) -> String   // Rubyはbaseのみ連結
  ```

- [ ] **Step 1: 失敗するテストを書く**
  - `parse_segs("'Codex|コーデックス'の非対話")` == `[Ruby{base:"Codex",reading:"コーデックス"}, Plain("の非対話")]`
  - `parse_segs("ab'cd")`(閉じ無し)== `[Plain("ab'cd")]`
  - `parse_segs("全角｜は区切りでない'a｜b'")` に `Ruby` を含まない
  - `parse_script("# c\n@script\n#paragraph[S01] オープニング\n[S01-L001] 蒼真:こんにちは\n")` が `[Scene{S01,オープニング}, Line{id:"S01-L001",scene:"S01",cast:"蒼真",display_text=="こんにちは"}]`
  - BOM先頭+CRLF入力でも同じ結果(Review Focus 3)。`@cast` 以降の説明行・`#visual` 行は結果に出ない。
- [ ] **Step 2:** `cargo test script` が失敗(未定義)することを確認。
- [ ] **Step 3:** 上記シグネチャを実装。行は正規表現相当で `^\[(S\d+)-L\d+\]\s*([^:]+):(.*)$`(正規表現crateは足さず手書き)。`:` は全角 `：` も区切りとして許可。
- [ ] **Step 4:** `cargo test script` 全PASS。
- [ ] **Step 5: Commit** `feat: 台本パーサ(行・シーン見出し・ルビ)`

### Task 2: タイムライン読込と台本の結合(`src/timeline.rs`)

**Files:** Create `src/timeline.rs`

**Interfaces:**
- Consumes: `script::{ScriptItem, ScriptLine, display_text}`
- Produces:
  ```rust
  pub struct TimelineLine { pub id: String, pub start_ms: u64, pub end_ms: u64, pub text: String, pub missing: bool }
  pub struct Timeline { pub script_sha256: String, pub duration_ms: u64, pub sample_rate: u32, pub lines: Vec<TimelineLine> }
  pub fn parse_timeline(json: &str) -> Result<Timeline, String>
  pub fn sha256_hex(bytes: &[u8]) -> String
  /// start_ms <= ms の最後の行(行間の無音では直前の行を保持)。無ければ None。missing行は除外。
  pub fn current_line(tl: &Timeline, ms: u64) -> Option<usize>
  pub struct Warnings(pub Vec<String>);
  /// 台本IDがtimelineに無い/missing行、表示文とtimeline.textの不一致をWarningsに積む
  pub fn check_consistency(items: &[ScriptItem], tl: &Timeline, script_bytes: &[u8]) -> Warnings
  ```

- [ ] **Step 1: テスト** 最小JSON(2行、うち1行 `missing:true`)で `parse_timeline` の件数/時刻、`current_line(tl, 13_000)==Some(0)`(行間保持)、`current_line(tl, 0)` 先頭、`missing` 行はスキップ。`sha256_hex(b"abc")=="ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"`。`check_consistency` は台本にあるがtimelineに無いID、およびsha不一致の警告文字列(「台本がbuild後に変更されています」を含む)を返す(Review Focus 1)。
- [ ] **Step 2:** 失敗確認。 **Step 3:** 実装(`serde_json`、不明フィールドは無視)。 **Step 4:** PASS確認。
- [ ] **Step 5: 実データ確認** `D:\UDS\YouTube\AI道具箱\第60回` を読み込む `#[test] #[ignore]` を追加し `cargo test -- --ignored timeline` が114行・936710msで通ること。
- [ ] **Step 6: Commit** `feat: timeline.json読込と整合性チェック`

### Task 3: 文字時刻の推定(`src/charmap.rs`)

**Files:** Create `src/charmap.rs`

**Interfaces:**
- Consumes: `script::Seg`
- Produces:
  ```rust
  pub struct CharSpan { pub start_ms: u64, pub end_ms: u64 }   // display_text の1文字ごと
  pub fn mora_count(reading: &str) -> f32
  pub fn char_spans(segs: &[Seg], start_ms: u64, end_ms: u64) -> Vec<CharSpan>
  pub fn char_index_at(spans: &[CharSpan], ms: u64) -> Option<usize>
  ```

- [ ] **Step 1: テスト** `mora_count("きょう")==2.0`、`mora_count("っ")==1.0`。`char_spans` の個数が `display_text` の文字数と一致、先頭 `start_ms`・末尾 `end_ms` が引数と一致、隙間なく連続、`Ruby{base:"東京",reading:"とうきょう"}` の2文字が読みのモーラ数(4)を等分した幅になる(各2文字とも同幅)、`。` の幅が `、` の約2倍。`char_index_at` は範囲外で `None`、境界で次の文字。空文字列の行は空Vec。
- [ ] **Step 2:** 失敗確認。 **Step 3:** 実装(重みは Global Constraints の値。Rubyのbase各文字は `mora_count(reading)/base文字数`)。 **Step 4:** PASS確認。
- [ ] **Step 5: Commit** `feat: 読みに基づく文字時刻推定`

### Task 4: 音声読込とメーター(`src/audio.rs`, `src/meter.rs`)

**Files:** Create `src/audio.rs`, `src/meter.rs`

**Interfaces:**
- Produces:
  ```rust
  // audio.rs
  pub struct AudioData { pub sample_rate: u32, pub channels: u16, pub samples: std::sync::Arc<Vec<f32>> }
  impl AudioData { pub fn frame_count(&self) -> u64; pub fn duration_ms(&self) -> u64 }
  pub fn load_wav(path: &std::path::Path) -> Result<AudioData, String>
  // meter.rs  (スペアナⅡ crates/sp2-dsp/src/meter.rs から analyze_meter / MeterFrame / ChannelMeter / METER_FLOOR_DBFS をコピー引用。find_file_peakは持ち込まない)
  pub fn analyze_meter(interleaved: &[f32], channel_count: u16, sample_rate_hz: u32, center_time_s: f32, window_duration_s: f32) -> MeterFrame
  ```

- [ ] **Step 1: テスト** `hound` で一時WAV(float32 48kHz 2ch、0.5s正弦波)を書き、`load_wav` が `channels==2`、`frame_count==24000`、`duration_ms==500` を返す。16bit整数WAVでも同様にf32化される。存在しないパスは `Err` でパスを含む。`analyze_meter`: 振幅1.0の定数で peak≈0 dBFS、無音で `METER_FLOOR_DBFS`、モノラル入力で1チャンネル。
- [ ] **Step 2:** 失敗確認。 **Step 3:** 実装(`hound::WavReader`、整数形式は正規化してf32、`samples` は `Arc`)。 **Step 4:** PASS確認。
- [ ] **Step 5: 実データ確認** `#[ignore]` テストで第60回の `full_dialogue.wav` が読め、`duration_ms` が936710±50であること(WAVE_FORMAT_EXTENSIBLE対応の確認。読めなければ `hound` 側の制約としてBeadsにblockerを起票しHumanへ返す)。
- [ ] **Step 6: Commit** `feat: WAV読込とメーター解析`

### Task 5: トランスポートと出力(`src/transport.rs`, `src/output.rs`)

**Files:** Create `src/transport.rs`, `src/output.rs`

**Interfaces:**
- Consumes: `audio::AudioData`
- Produces:
  ```rust
  pub const FAST_RATE: f64 = 4.0;  pub const SKIP_MS: u64 = 10_000;
  #[derive(Clone, Copy, PartialEq, Eq, Debug)]
  pub enum Mode { Stopped, Play, Forward, Rewind }
  pub struct Transport { /* pos_frames: f64, total_frames, sample_rate, mode */ }
  impl Transport {
      pub fn new(total_frames: u64, sample_rate: u32) -> Self;
      pub fn mode(&self) -> Mode;   pub fn set_mode(&mut self, m: Mode);
      pub fn position_ms(&self) -> u64;   pub fn seek_ms(&mut self, ms: i64);  // 0..=total にクランプ
      pub fn skip_ms(&mut self, delta_ms: i64);
      /// 出力バッファを埋め、位置を進める。端に達したら Stopped にして残りは無音。
      pub fn render(&mut self, samples: &[f32], channels: u16, out: &mut [f32]);
  }
  // output.rs
  pub struct Output { /* cpal::Stream を保持 */ }
  pub fn start_output(audio: AudioData, transport: std::sync::Arc<std::sync::Mutex<Transport>>) -> Result<Output, String>
  ```

- [ ] **Step 1: テスト**(`Transport::render` を直接検証)
  - 1kHz・ステレオの連番サンプルで、`Play` の1回の `render(…, 100フレーム)` 後に `position_ms` が100フレーム分進み、出力が元サンプルと一致。
  - `Forward` は同じ出力フレーム数で 4倍進む。`Rewind` は位置が減り、出力が逆順サンプル(Review Focus 4)。
  - 位置0で `Rewind` → 次の `render` で `Stopped` になり位置0、出力無音。末尾で `Play` → `Stopped`、位置=総フレーム。
  - `seek_ms(-5)` は0、`seek_ms(総時間+1)` は総時間。`skip_ms(10_000)`/`skip_ms(-10_000)` が±10秒でクランプ。`Stopped` の `render` は無音で位置不変。
- [ ] **Step 2:** 失敗確認。 **Step 3:** `Transport` 実装(高速時は最近傍サンプル、逆再生は位置を減算しながら読む)。**Step 4:** PASS確認。
- [ ] **Step 5:** `output.rs`: `cpal` 既定出力デバイスに f32/i16/u16 のいずれかで接続し、コールバックで `try_lock` して `render`(ロック失敗時は無音)。サンプルレート/チャンネル不一致はデバイスの設定に合わせて `AudioData` を1フレーム単位で変換(単純な線形補間リサンプルは行わず、デバイスが48kHz非対応なら `Err` で理由を返す)。ストリームエラーは `Arc<AtomicBool>` で通知。
- [ ] **Step 6:** 実機確認: `examples/play.rs`(10秒再生して終了)で第60回が鳴ること、**目視・耳で確認**。
- [ ] **Step 7: Commit** `feat: トランスポートとcpal出力`

### Task 6: アプリ骨格・フォント・操作パネル(`src/app.rs`, `src/fonts.rs`, `src/ui/controls.rs`)

**Files:** Create `src/app.rs`, `src/fonts.rs`, `src/ui/mod.rs`, `src/ui/controls.rs`; Modify `src/main.rs`, `src/lib.rs`

**Interfaces:**
- Consumes: `transport::{Transport, Mode, SKIP_MS}`, `output::start_output`
- Produces:
  ```rust
  pub fn install_japanese_font(ctx: &egui::Context) -> Result<(), String>   // fonts.rs
  pub struct Project { pub script_path: PathBuf, pub items: Vec<ScriptItem>, pub timeline: Timeline, pub audio: AudioData, pub warnings: Warnings }
  pub fn load_project(script_path: &Path) -> Result<Project, String>   // app.rs: txt/timeline.json/full_dialogue.wav を同フォルダから(欠落はパス付きErr)
  pub fn draw_controls(ui: &mut egui::Ui, tr: &mut Transport, total_ms: u64)   // ui/controls.rs
  pub struct App { /* project: Option<Project>, transport: Arc<Mutex<Transport>>, output: Option<Output>, follow: bool, show_ruby: bool */ }
  ```

- [ ] **Step 1: テスト** `load_project` に存在しないフォルダを渡し、Errに `full_dialogue.wav` か `timeline.json` のパスが含まれる。一時フォルダに最小のtxt/json/wavを置いたら `Ok`(Task 2・4の関数を使用)。
- [ ] **Step 2:** 失敗確認→`load_project` 実装→PASS。
- [ ] **Step 3:** `fonts.rs`: `YuGothM.ttc` → `meiryo.ttc` → `msgothic.ttc` の順に `C:\Windows\Fonts\` から読み、`egui::FontDefinitions` の Proportional/Monospace 先頭に追加。全て無ければ `Err`。
- [ ] **Step 4:** `controls.rs`: 6ボタン(⏪ ◀10s ■ ▶ 10s▶ ⏩、再生中モードのボタンは選択表示)と、その下に `egui::Slider`(0..=total_ms、ドラッグ中は `seek_ms`)+ `mm:ss.s / mm:ss.s` 表示。早戻し=`Mode::Rewind`、早送り=`Mode::Forward`、停止=`Mode::Stopped`、▶=`Mode::Play`、10秒系=`skip_ms(±SKIP_MS as i64)`。
- [ ] **Step 5:** `App`: 起動引数(台本パス)または「台本を開く」ボタン(`rfd`)で `load_project`、成功時に `start_output`。下パネルに操作、中央に台本(Task 7)、右にメーター(Task 8)の配置枠を用意。エラーと警告は上部に赤/黄で表示。`ctx.request_repaint()` で再生中は継続再描画。
- [ ] **Step 6:** `cargo run --release -- "D:\UDS\YouTube\AI道具箱\第60回\第60回.txt"` で、日本語が豆腐にならず、6ボタンとシークバーが動き、音が出ることを確認(Review Focus 2)。
- [ ] **Step 7: Commit** `feat: アプリ骨格・日本語フォント・操作パネル`

### Task 7: 台本表示(ルビ併記・行強調・文字追跡)(`src/ui/script_view.rs`)

**Files:** Create `src/ui/script_view.rs`; Modify `src/app.rs`

**Interfaces:**
- Consumes: `timeline::current_line`, `charmap::{char_spans, char_index_at}`, `script::{ScriptItem, Seg}`
- Produces:
  ```rust
  pub enum LineState { Past, Current { char: Option<usize> }, Future }
  /// 折り返し込みで1行を描き、行全体のRectを返す。ルビは語の上に0.55倍サイズ。
  pub fn draw_line(ui: &mut egui::Ui, line: &ScriptLine, state: LineState, show_ruby: bool) -> egui::Response
  /// 折り返し配置の純ロジック: 各トークン(1文字 or ルビ語)の幅リストと利用幅から、行ごとのトークン範囲を返す
  pub fn wrap_tokens(widths: &[f32], max_width: f32) -> Vec<std::ops::Range<usize>>
  pub fn draw_script(ui: &mut egui::Ui, items: &[ScriptItem], tl: &Timeline, pos_ms: u64, follow: bool, show_ruby: bool) -> Option<u64>  // 行クリック時にその行のstart_msを返す
  ```

- [ ] **Step 1: テスト** `wrap_tokens`: 幅[10;7]、max 30 → `[0..3, 3..6, 6..7]`、1トークンがmaxを超えても無限ループせず1トークン1行で進む。
- [ ] **Step 2:** 失敗確認→`wrap_tokens` 実装→PASS。
- [ ] **Step 3:** `draw_line`: トークン = Plainは1文字ずつ、Rubyは語1つ(幅 = max(base幅, ruby幅))。`Ruby`の読みを語の上中央に描画。`show_ruby == false` ではルビ行を描かない。`Current` は行背景を強調色、`char` 以前の文字は濃色・現在文字は下線+背景、以降は通常色(Rubyは語のbase範囲に現在文字が入ったら語全体を現在扱い)。話者名は行頭に固定幅で表示。
- [ ] **Step 4:** `draw_script`: シーン見出し(`S01 オープニング`)を区切りとして描画、`ScrollArea::vertical` 全高。現在行は `current_line` で決定し(`Current`)、それ以前=Past、以降=Future。`follow` が真で現在行が変わったときだけ `scroll_to_rect`。行クリックでその行の `start_ms` を返し、`App` が `seek_ms`。
- [ ] **Step 5:** 実機確認(第60回): 再生で行強調が切り替わり、`'Codex|コーデックス'` がルビ付き表示、ルビ切替が効き、行クリックでシークし、5分以降へシークしても追従することを確認。
- [ ] **Step 6: Commit** `feat: ルビ併記の台本表示と行・文字追跡`

### Task 8: 密着メーター(`src/ui/meter_view.rs`)

**Files:** Create `src/ui/meter_view.rs`; Modify `src/app.rs`

**Interfaces:**
- Consumes: `meter::{analyze_meter, MeterFrame}`
- Produces:
  ```rust
  pub const METER_WINDOW_S: f32 = 0.05;
  pub struct MeterViewState { /* peak_hold_dbfs: Vec<f32> */ }
  impl MeterViewState { pub fn update(&mut self, m: &MeterFrame); pub fn reset(&mut self); pub fn hold(&self) -> &[f32] }
  pub fn meter_y_fraction(dbfs: f32) -> f32     // -60→0.0, 0→1.0、範囲外はクランプ
  pub fn draw_meter(ui: &mut egui::Ui, state: &mut MeterViewState, m: &MeterFrame)
  ```

- [ ] **Step 1: テスト** `meter_y_fraction(-60.0)==0.0`、`(0.0)==1.0`、`(-30.0)==0.5`、`(-90.0)==0.0`、`(6.0)==1.0`。`update` がホールドを最大値で保持し、`reset` で消える。
- [ ] **Step 2:** 失敗確認→実装→PASS。
- [ ] **Step 3:** `draw_meter`: スペアナⅡ `meter_view.rs` を参考に、目盛列(-60,-48,-36,-24,-12,0)+L/Rバーを**隙間なく隣接**させて描画(全体幅は約90px固定)。RMSは塗り、ピークは青線、ホールドは赤線、バー下にピーク数値のみ(1行)。`SidePanel::right(..).exact_width(..)` に配置し、シーク/停止/新規ロードで `reset`。
- [ ] **Step 4:** `App` で毎フレーム `analyze_meter(samples, ch, sr, pos_s, METER_WINDOW_S)` を呼び、表示。実機で第60回の再生中にレベルが動き、台本エリアが画面の過半を占めることを確認。
- [ ] **Step 5: Commit** `feat: 密着レベルメーター`

### Task 9: 仕上げ・総合確認・レビュー依頼

**Files:** Modify `src/app.rs`, 新規 `README.md`

- [ ] **Step 1:** 警告表示(台本不一致・音声長差>1秒・欠落行件数)を `App` 上部に常時表示。音声長差は `audio.duration_ms()` と `timeline.duration_ms` の比較を `load_project` で `Warnings` に積む(テスト: 差2000msで警告が出る/差0で出ない)。
- [ ] **Step 2:** `README.md` に起動方法(`cargo run --release -- <台本.txt>`)と操作を記述。
- [ ] **Step 3:** `cargo test` 全PASS、`cargo clippy -- -D warnings` クリーン、`cargo build --release` 成功。
- [ ] **Step 4:** 設計書 §9 完了条件1〜5を実機(第60回)で1つずつ確認し、結果をBeads issueにコメントとして記録。
- [ ] **Step 5:** 独立レビュー(実装者と別のレビュアー)を依頼し、指摘対応後にHumanへ報告。issueのcloseはレビュー後。
- [ ] **Step 6: Commit** `docs: README・総合確認`
