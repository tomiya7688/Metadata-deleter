# Metadata deleter (`metaclean`)

PNG / JPEG / WebP の作成者・位置情報・使用機器・制作履歴などを、**ローカル**で削除・選択的に置換するRust製CLIです。画像の圧縮データは再エンコードしません。

**重要**：すべての個人情報の消去を保証するものではありません。画像そのものに写っている情報、未知の独自形式、残したコンテナ内の情報、ファイルシステムのタイムスタンプやACLなどは別です。初めは `--copy` で確認してください。

## 配布とビルド

GitHub Actionsの [Rust CI](https://github.com/tomiya7688/Metadata-deleter/actions/workflows/ci.yml) の成功した実行の **Artifacts** から、Windows / Linux の実行ファイルと `metaclean.ini` が入ったパッケージを入手できます。WindowsではRustやPythonをインストールせずに使えます。

ソースからビルドする場合：

```sh
cargo test --all-targets
cargo build --release
```

依存クレートはありません。アプリケーションのRustコードは `#![forbid(unsafe_code)]` を使用しており、`unsafe` ブロックや独自のWin32 FFIを使用しません。

## INIの読み込み先（重要）

`--config` を指定しない場合、**`metaclean` 実行ファイルのあるディレクトリの `metaclean.ini`** を読み込みます。処理を開始したカレントディレクトリや画像フォルダからは自動検索しません。

```text
C:\Tools\MetaClean\
  metaclean.exe
  metaclean.ini
```

- 実行ファイルと同じフォルダにINIがあれば、それを読む（読み込み・構文エラーなら処理中止）。
- INIがなければ、実行ファイルに埋め込まれたデフォルト設定を使う。
- `--config C:\path\custom.ini` を指定すれば、こちらが最優先（相対パスはカレントディレクトリ基準）。
- `metaclean --print-default-config` でデフォルト設定を表示できる。

## 基本操作

```powershell
metaclean photo.jpg --in-place  # 安全な一時ファイルを使って上書き
metaclean photo.jpg --copy      # photo_clean.jpg を新規作成
metaclean ./photos --in-place  # サブフォルダを含めて上書き
metaclean ./photos --copy      # photos_clean/ にフォルダ構成を維持してコピー
```

コピー先がすでに存在すれば上書きしません。フォルダコピーでは未対応拡張子もそのままコピーし、シンボリックリンクはたどりません。一部ファイルの処理に失敗した場合はエラーを返しますが、すでに処理できた別のファイルを巻き戻しません。

## 設定と置換

```ini
[keep]
creator = "tomiya"
contact = false
gps = false
camera_make = false
camera_model = false
device_serial = false
software = false
color_profile = true
orientation = true
resolution = true
```

- `true`：その分類に該当する既存の項目を保持。
- `false`：該当項目を削除。
- `"任意の文字列"`：**置換対応のテキスト項目**に限り指定値に書き換える。引用符内の `true` / `false` も文字列扱い。
- 引用符のない `tomiya`、シングルクォーテーション、文字列に非対応の項目への置換はエラー。

置換は**既存の対象項目**に対して行い、元画像にメタデータがなければ新規には挿入しません。EXIFの文字列はフォーマット上の制約があり、UnicodeはXPAuthor・XPTitle等のUTF-16項目へ移す場合があります。文字列値の一部はEXIF ASCIIフィールドで置換不能なため、エラーになります。PNGでは置換対象の元チャンクを非圧縮iTXtに再構成します。

`exif=true` / `xmp=true` / `iptc=true` は**コンテナ丸ごとの保持**です。その内部に削除予定の個人情報があっても残ります。誤解を防ぐため、これらと文字列置換の同時指定はエラーになります。

### 全設定項目

| キー | 初期値 | 処理の概要 | `"文字列"` |
|---|---|---|---|
| `exif` | `false` | EXIF全体（false時は許可項目を安全に再構成） | × |
| `xmp` | `false` | XMP/XML全体（false時は指定した置換値のみ再構成） | × |
| `iptc` | `false` | JPEG APP13 Photoshop/IPTCコンテナ | × |
| `creator` | `false` | 作成者、EXIF Artist/XPAuthor/CameraOwnerName、PNG Author | ○ |
| `copyright` | `false` | 権利者名、著作権表示 | ○ |
| `software` | `false` | 編集アプリ・生成ツール | ○ |
| `comments` | `false` | JPEG COM、EXIF/PNGのコメント | ○ |
| `timestamps` | `false` | 撮影・制作日時、EXIFタイムゾーン・PNG tIME等 | × |
| `ai_generation` | `false` | PNGの生成プロンプト／シード／ワークフロー等 | ○ (PNG) |
| `provenance` | `false` | C2PA/JUMBF: PNG caBX、WebP C2PA、JPEG APP11 | × |
| `title` | `true` | タイトル、XPTitle、XMP dc:title、PNG Title等 | ○ |
| `description` | `true` | EXIF ImageDescription、PNG Description等 | ○ |
| `keywords` | `true` | タグ、XPKeywords、PNG Keywords、XMP dc:subject | ○ |
| `contact` | `false` | PNGのEmail・Website・Phoneなどの連絡先 | ○ (PNG) |
| `gps` | `false` | EXIF GPS IFD、PNG GPS/Location/Latitude等 | × |
| `camera_make` | `false` | EXIF Make、PNG Make | ○ |
| `camera_model` | `false` | EXIF Model、PNG Source/Camera/Model | ○ |
| `lens` | `false` | EXIF LensMake/LensModel、PNG Lens等 | ○ |
| `device_serial` | `false` | EXIF BodySerialNumber/LensSerialNumber等 | ○ |
| `camera_settings` | `false` | EXIF ISO、露出、焦点距離、PNGカメラ設定等 | × |
| `image_id` | `false` | EXIF ImageUniqueID、PNG Document ID等 | ○ |
| `rating` | `true` | EXIF Rating/RatingPercent、PNG Rating等 | × |
| `jpeg_thumbnail` | `false` | JPEG APP0 JFXXプレビュー（EXIF埋め込みサムネイルは常に除去） | × |
| `mpf` | `true` | JPEG APP2 MPFマルチ画像索引（下記注意） | × |
| `flashpix` | `false` | JPEG APP2 FPXR | × |
| `ducky` | `false` | JPEG APP12 Ducky | × |
| `png_extension` | `true` | PNGのoFFs / sCAL / pCAL / vpAg等の拡張付加情報 | × |
| `color_profile` | `true` | ICC、色空間、ガンマなど | × |
| `orientation` | `true` | EXIF Orientation（回転表示） | × |
| `resolution` | `true` | EXIF解像度、PNG pHYs、JPEG JFIF密度 | × |
| `other_text` | `true` | まだ分類していないPNGテキスト | ○ (PNG) |
| `unknown` | `true` | まだ認識していない付加チャンク・JPEG APPセグメント | × |

`true` は**個人情報が存在しないという意味ではありません**。たとえばタイトルや未知のテキストに名前が書かれている可能性があります。公開用に厳格に処理したい場合は必要に応じて `title=false`、`description=false`、`keywords=false`、`other_text=false`、`unknown=false` などを選んでください（未知の付加情報は表示にも影響する可能性があります）。

### コンテナ固有の制約

- **PNGの旧形式**：ExifTool/ImageMagick/GIMPなどで使われる `Raw profile type exif` / `APP1` / `8bim` / `iptc` は、個人情報が残りやすいため、通常の `other_text=true` に依存せず `exif` / `iptc` 設定で分類します。`Raw profile type icc/icm` は色プロファイル扱いです。
- **EXIF**：`exif=false` では既知のIFD0、Exif SubIFD、GPS IFDタグのみを書き出します。MakerNote、未知のEXIFタグ、Exif IFD1の埋め込みサムネイルなどは常に破棄されます。`unknown=true` にしても、これらが暗黙に残るわけではありません。GPS日時は `gps=true` でも `timestamps=false` なら残しません。
- **XMP**：パーサーによる未知XML要素ごとの取捨選択ではなく、コンテナ全体の除去と、明示的に指定した一部の置換値の新規XMPへの再生成です。`title=true` 等にしても、元のXMPの該当要素だけを自動抽出して保持するわけではありません。
- **IPTC**：JPEG APP13を一括保持・削除します。個々のIPTCタグの意味的な置換は行いません。
- **MPF JPEG**：MPF内のマルチ画像参照オフセットは、ExifやAPPセグメントを削除すると壊れる可能性があります。`mpf=true` のままファイル内容が変わる場合は**安全のためエラーで停止**します。`mpf=false` にすると索引と、後続JPEG画像を失う可能性があります。
- **C2PA**：C2PA署名はメタデータ書換えで無効になるため、通常は `provenance=false` を推奨します。

### 安全性

処理後に構造を再解析し、同じ設定でもう一度処理したとき同一バイト列になることを検証します。PNGではCRCを検証し、書き換えたチャンクのCRCを再計算します。JPEGの圧縮ストリーム、PNGのIDAT、WebPのVP8/VP8Lフレームには触れません。

上書き時は同じディレクトリの一時ファイルから`std::fs::rename`で置換し、元画像への部分書き込みを防ぎます。コピーは一時ファイルへの書込み後、ハードリンクによる新規公開を試み、既存パスを上書きしません。ハードリンク非対応ファイルシステムではエラーになる場合があります。OSの拡張属性や作成日時が維持されることは保証しません。
