# Metadata deleter (`metaclean`)

PNG・JPEG・WebP画像の**出自に関するメタデータ**をローカルで除去する、Rust製の小さなCLIツールです。
画像データは**再エンコードしません**。Pythonやネット接続、外部プログラムも不要です。

> **範囲について**：すべての個人情報が消えることを保証するツールではありません。画像に写り込んだ情報、未知の独自チャンク、保持を選んだICCプロファイル等、ファイルシステムの属性には情報が残る場合があります。公開前に必要に応じてご確認ください。

## ビルド

Rustのツールチェーンが必要です（利用時は不要）。

```sh
cargo test
cargo build --release
```

ビルド後の実行ファイルはWindowsでは `target/release/metaclean.exe`、Linux/macOSでは `target/release/metaclean` です。

GitHub Actionsの [`Rust CI`](https://github.com/tomiya7688/Metadata-deleter/actions/workflows/ci.yml) が成功すると、実行履歴の **Artifacts** から `metaclean-Windows` と `metaclean-Linux` の単独実行ファイルを取得できます（アーカイブを展開して利用してください）。

## 4つの操作

```powershell
# ファイルを安全に上書き
metaclean photo.jpg --in-place

# photo_clean.jpg を新規作成（同名が存在したら停止）
metaclean photo.jpg --copy

# フォルダ以下の対応画像を再帰的に上書き
metaclean ./photos --in-place

# photos_clean/ を新規作成しフォルダ構成を維持
metaclean ./photos --copy
```

フォルダコピーでは未対応ファイルも**変更せずコピー**します。上書き処理では未対応形式をスキップします。シンボリックリンクと特殊ファイルはたどらず、コピーもしません。フォルダで一部ファイルが失敗した場合、残りのファイルも処理し、終了コードを `1` にします。途中まで処理されたファイルやコピー先フォルダは自動で巻き戻しません。

## 設定 (INI)

`metaclean.ini` をカレントディレクトリに置くと自動読み込みします。設定がなければ以下と同じ安全側の初期値を使います。別の場所から読み込む場合は `--config path/to/config.ini` を付けます。`metaclean --print-default-config` でも初期値を確認できます。

**`true` = 保持、`false` = 削除**。設定値はすべて真偽値です。

```ini
[keep]
exif = false
xmp = false
iptc = false
comments = false
creator = false
copyright = false
software = false
timestamps = false
ai_generation = false
provenance = false
color_profile = true
orientation = true
resolution = true
other_text = true
unknown = true
```

| 設定 | 対象（概要） | デフォルト |
|---|---|---|
| `exif` | EXIFコンテナ全体。`false` 時は回転・解像度の値だけ再構成可能 | `false` |
| `xmp` | XMPコンテナ（独自XMP拡張も含む） | `false` |
| `iptc` | JPEG Photoshop APP13領域（IPTC等を含む） | `false` |
| `comments` | JPEG COM、PNGのComment/Description等のテキスト | `false` |
| `creator` | PNGのAuthor/Artist/Creator等のテキスト | `false` |
| `copyright` | PNGのCopyright/Rights holder等のテキスト | `false` |
| `software` | PNGのSoftware/Tool/Encoder等のテキスト | `false` |
| `timestamps` | PNGのCreation Time等、`tIME` | `false` |
| `ai_generation` | PNGのparameters/Prompt/Workflow/Seed等 | `false` |
| `provenance` | JPEG APP11 JUMBF/C2PA識別領域 | `false` |
| `color_profile` | ICC等の色表示用情報 | `true` |
| `orientation` | EXIFの回転方向（EXIFを除去するときだけ適用） | `true` |
| `resolution` | EXIFの解像度、PNG pHYs、JPEG JFIF密度 | `true` |
| `other_text` | PNGで分類できないテキストキーワード | `true` |
| `unknown` | 未分類の付加チャンクやJPEG APP領域 | `true` |

**コンテナごとの制約**：EXIF・XMP・IPTCを `true` にすると、そのコンテナ内のGPSや作成者情報等も保持されます。`exif=false` はEXIFのタグを一部削除するのではなく、`orientation=true` / `resolution=true` で必要な表示用タグのみを再構成します。JPEGの写真編集情報やXMPの著作権情報など、**個人情報以外が同じコンテナにあると一緒に削除される**場合があります。設定はコンテナ単位とPNGキーワード単位の組み合わせです。

未知の付加情報はデフォルトで保持するため、既知でないメタデータが残る可能性があります。未知の非必須チャンクを除去する場合は `unknown = false` にできますが、独自形式で使用される付加情報まで失われることがあり、画像表示に影響するリスクがあります。表示の維持と未知情報の消去の完全な両立は保証できません。

## 対応形式と処理方式

- **PNG**：チャンク単位でテキスト・日時・eXIf等を除去。CRCを検証し、再構成したチャンクのCRCを更新。IDAT、パレット、透明度、APNGフレームを保持。
- **JPEG**：Exif/XMP/Photoshop APP13/COM等のセグメントを除去。EXIFのOrientation・解像度は小さなEXIF領域として再構成。SOS以降のエントロピー符号化データはそのまま保持。EOI以降の余剰データは出力しません。
- **WebP**：RIFF EXIF/XMPチャンクを除去し、VP8XのEXIF/XMP/ICCフラグとRIFFサイズを更新。VP8/VP8L・アニメーション画像データを保持。

画像の**ファイル構造**を検証しますが、画像デコーダを使ってピクセルを全展開し、完全にデコード可能かを保証するわけではありません。メタデータ削除後は同じ設定で再解析し、変更が再発しないことを検証します。

## 安全上の動作

- 上書き時は同じディレクトリの一時ファイルに全データを書き出し、検証後にファイルを置換します（Windowsでは `ReplaceFileW`、その他のOSではrename）。処理途中の書き込みで元画像を直接壊しません。
- コピー先のファイルやフォルダが既に存在する場合は上書きしません。
- PNG/JPEG/WebPとして認識できないファイル、構造が壊れたファイルは無理に編集しません。
- ファイル名の拡張子で対象を判断します（`.png`, `.jpg`, `.jpeg`, `.webp`、大文字も対応）。
- コピー先フォルダは作成済みかどうかを確認して新規作成します。1ファイル単位での安全性は確保しますが、フォルダ全体を1つのトランザクションとして扱いません。
- 出力ファイルには元の通常のファイルパーミッションを可能な範囲で引き継ぎますが、すべての拡張属性・ACL・ファイルシステム上のタイムスタンプを保持／消去するものではありません。

既知のコンテナ以外に埋め込まれたプライバシー情報、サムネイルなどに画像として写り込んだ情報、カラープロファイル内部の記述文字列、クラウドサービスやOSが保持する作成記録は対象外です。

## 開発

依存クレートは使っていません。構成は `src/config.rs`（INI）、`src/ops.rs`（CLIのファイル操作）、`src/formats/`（PNG/JPEG/WebP）、`src/exif.rs`（表示用EXIFの最小再構成）です。CIではUbuntu/Windowsで `cargo test --all-targets` とリリースビルドを実行します。
