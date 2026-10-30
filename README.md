# ScriptChecker

`full_dialogue.wav` を台本・`timeline.json` と同期して確認する、Windows向けのデスクトップツールです。

## 起動

Rust をインストールした状態で、台本ファイルを指定します。

```powershell
cargo run --release -- "D:\UDS\YouTube\AI道具箱\第60回\第60回.txt"
```

台本と同じフォルダに `full_dialogue.wav`、`timeline/timeline.json` が必要です。起動後の「台本を開く」からも選択できます。

## 操作

- `⏪`: 4倍速の逆再生
- `◀ 10s`: 10秒戻す
- `■`: 停止
- `▶`: 通常再生
- `10s ▶`: 10秒送る
- `⏩`: 4倍速の早送り再生
- シークバー: 指定位置へ移動
- 台本の行をクリック: その行の先頭へ移動
- `追従` / `ルビ表示`: 自動追従・読み仮名を切り替え

`timeline.json` と台本のハッシュ、行ID、音声長に不一致があっても再生は許可し、画面上に警告を表示します。
