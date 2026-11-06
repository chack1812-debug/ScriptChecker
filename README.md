# ScriptChecker

YouTube 番組制作の公開前チェックを助ける、Windows 向けのデスクトップツールです。
[Script2Video](https://github.com/chack1812-debug/Script2Video) が作った台本・音声・タイムラインを読み込み、音声の再生に合わせて台本の該当行を強調します。主な用途は、**音声レベルの確認**と、**誤読・ルビ(読み指定)の誤りの確認**です。

- 再生中の行を背景色で強調し、行内の読み位置を文字色で示す(行は正確、文字は推定の目安)
- 読み指定 `'単語|読み'` のある語の上に、読みを小さなルビ文字で併記(ON/OFF 切替)
- 音声のレベルメーター(RMS / ピーク / ピークホールド、数値表示つき)
- 6 つの再生操作と、全幅のシークバー
- 台本の文字を選択してコピーできる(台本の修正指示に使えます)

## ダウンロード

ビルド済みの実行ファイル(Windows)は、[宇摩コードワークスのサイト](https://umacodeworks.jp/scriptchecker.html)で配布しています(GitHub には zip を置いていません)。

- 配布ページ: <https://umacodeworks.jp/scriptchecker.html>
- v0.1.21 の zip(直接): <https://umacodeworks.jp/downloads/ScriptChecker_v0.1.21.zip>

zip には `scriptchecker.exe`・`README.md`・`LICENSE`・`THIRD-PARTY-NOTICES.txt` が入っています。展開して `scriptchecker.exe` を実行するだけで使えます(インストール不要)。配布用 zip は `scripts\package.ps1` で作れます。

## 必要なもの

台本ファイルと同じフォルダに、以下が必要です。

```
第N回.txt                 台本。[S01-L001] 話者:台詞 の行と #paragraph[S01] 見出し を含む
full_dialogue.wav         全体の音声
timeline/timeline.json    各行の開始・終了時刻(Script2Video の build が作る)
```

ビルドには Rust(Edition 2024 に対応した版)が必要です。

## 起動

```powershell
cargo run --release
```

起動するとファイル選択ダイアログが開くので、台本(`第N回.txt`)を選びます。台本のパスを引数に指定すると、ダイアログなしで開きます。

```powershell
cargo run --release -- "D:\UDS\YouTube\AI道具箱\第60回\第60回.txt"
```

起動後は、右上の「台本を開く」から別の台本を開けます。リリースビルドでは、黒いコンソール画面は出ません。

## 操作

| 操作 | 内容 |
|---|---|
| `⏪` | 4 倍速の逆再生 |
| `◀ 10s` | 10 秒戻す |
| `■` | 停止(位置を保持) |
| `▶` | 通常再生 |
| `10s ▶` | 10 秒送る |
| `⏩` | 4 倍速の早送り再生 |
| シークバー | 指定位置へ移動 |
| 台本の行をクリック | その行の先頭へ移動 |
| `追従` / `ルビ表示` | 自動スクロール・ルビの表示を切り替え |
| `Reset peak` | ピークホールドを消す(メーター下部) |
| ドラッグ | 台本の文字を選択 |
| `Ctrl+C` / 右クリック | 選択範囲、またはその行をコピー |

コピーした行は `[S01-L001] 蒼真:本文` の形式です。ルビ付きの語は `'単語|読み'` のまま入ります。

## 警告表示

次の場合でも再生はできますが、画面上部に警告を表示します。

- 台本が `timeline.json` の作成後に変更されている(SHA-256 の不一致)
- 台本の行が `timeline.json` に見つからない、または欠落している
- 音声の長さが `timeline.json` と大きく違う

`full_dialogue.wav` や `timeline.json` が見つからない場合は、パスを示すエラーを表示します。

## 仕様の補足

- 画面の強調・メーター・シークバー・停止位置は、出力デバイスの遅延を補正した「聞こえている位置」を基準にしています。
- メーターの解析は、現在位置を中心にした 0.3 秒の窓で行い、停止中も表示を続けます。ピークホールドは、停止やシークでは消えません。
- 文字単位の強調は、行の時間を読みの長さに比例して割り振った推定です。数百ミリ秒程度のずれは仕様です。

設計の詳細は [`docs/superpowers/specs/2026-10-30-scriptchecker-design.md`](docs/superpowers/specs/2026-10-30-scriptchecker-design.md) にあります。

## 開発

```powershell
cargo fmt --check
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
```

第 60 回の実データを使うテストは `#[ignore]` になっています。実行するには `cargo test -- --ignored` を使います。

## ライセンス

[GNU General Public License v3.0 以降](LICENSE)(GPL-3.0-or-later)。
