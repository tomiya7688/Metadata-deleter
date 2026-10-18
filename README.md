# Metadata deleter (`metaclean`)

PNG・JPEG・WebP画像の**出自に関するメタデータ**をローカルで削除・書き換えする、Rust製の小さなCLIツールです。
画像データは**再エンコードしません**。Pythonやネット接続、外部プログラムも不要です。

> **範囲について**：すべての個人情報が消えることを保証するツールではありません。画像に写り込んだ情報、未知の独自チャンク、保持を選んだICCプロファイル等、ファイルシステムの属性には情報が残る場合があります。公開前に必要に応じてご確認ください。

## ビルド

Rustのツールチェーンが必要です（利用時は不要）。

```sh
cargo test
cargo build --release
```

ビルド後の実行ファイルはWindowsでは `target/release/metaclean.exe`、Linux/macOSでは `target/release/metaclean` です。

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

**INIの設定値は3種類**です。値の型は真偽値だけではありません。

- `true`（クォーテーションなし）：元の値を保持
- `false`（クォーテーションなし）：元の値を削除
- `"任意の文字列"`（必ず**ダブルクォーテーション**で囲む）：指定した文字列に置換

たとえば、過去のGoogleアカウント名などを新しい表示名へ変更したい場合は、`creator = "tomiya"` と指定します。

```ini
[keep]
creator = "tomiya"
comments = false
```

`creator = tomiya` や `creator = 'tomiya'` はエラーになります。`creator = "false"` は文字列 `false` への置換で、削除を意味しません。値に `;`、`#`、`=` を含める場合はダブルクォーテーションで囲みます。文字列内の `"` と `\` はそれぞれ `\"` と `\\` でエスケープできます。置換は対象となるメタデータ領域が存在する場合に行い、作成者情報が全くない画像へ無条件に作成者を追加するわけではありません。

**文字列置換に対応する設定**：`creator`、`copyright`、`software`、`comments`、`ai_generation`、`other_text`。それ以外（EXIF/XMP/IPTCコンテナ、位置付け用データ、色、向き、解像度、日時、未知のバイナリ領域）は `true` / `false` のみ有効です。対応外の項目に引用符付きの値を指定するとエラーで停止します。

**置換時の安全策**：`exif=true`、`xmp=true`、`iptc=true` と文字列置換は併用できません。元のコンテナを丸ごと保持すると旧名が別のタグに残るおそれがあるため、設定エラーにします。通常は3項目ともデフォルトの `false` を使用してください。

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
| `exif` | EXIFコンテナ全体。`false` 時は表示用の回転・解像度と、保持／置換を許可した一部のテキストフィールドを再構成 | `false` |
| `xmp` | XMPコンテナ（独自XMP拡張も含む） | `false` |
| `iptc` | JPEG Photoshop APP13領域（IPTC等を含む） | `false` |
| `comments` | JPEG COM、PNGのComment/Description、XMPの説明（置換時） | `false` |
| `creator` | PNGのAuthor/Artist/Creator、EXIF Artist/XPAuthor/CameraOwnerName、XMP dc:creator（置換時） | `false` |
| `copyright` | PNGのCopyright等、EXIF Copyright、XMP dc:rights（置換時） | `false` |
| `software` | PNGのSoftware等、EXIF Software、XMP CreatorTool（置換時） | `false` |
| `timestamps` | PNGのCreation Time等、`tIME` | `false` |
| `ai_generation` | PNGのparameters/Prompt/Workflow/Seed等 | `false` |
| `provenance` | JPEG APP11 JUMBF/C2PA識別領域 | `false` |
| `color_profile` | ICC等の色表示用情報 | `true` |
| `orientation` | EXIFの回転方向（EXIFを除去するときだけ適用） | `true` |
| `resolution` | EXIFの解像度、PNG pHYs、JPEG JFIF密度 | `true` |
| `other_text` | PNGで分類できないテキストキーワード | `true` |
| `unknown` | 未分類の付加チャンクやJPEG APP領域 | `true` |

**コンテナごとの制約**：EXIF・XMP・IPTCを `true` にすると、そのコンテナ内のGPSや作成者情報等も保持されます。`exif=false` ではEXIFの表示用タグおよび許可されたIFDテキストタグだけを再構成します。JPEGの写真編集情報やXMPの著作権情報など、**個人情報以外が同じコンテナにあると一緒に削除される**場合があります。設定はコンテナ単位とPNGキーワード単位の組み合わせです。

未知の付加情報はデフォルトで保持するため、既知でないメタデータが残る可能性があります。未知の非必須チャンクを除去する場合は `unknown = false` にできますが、独自形式で使用される付加情報まで失われることがあり、画像表示に影響するリスクがあります。表示の維持と未知情報の消去の完全な両立は保証できません。

## 対応形式と処理方式

- **PNG**：テキストの削除・置換（置換後は非圧縮UTF-8のiTXt）、日時・eXIf等を処理。CRCを検証・更新。IDAT、パレット、透明度、APNGフレームを保持。
- **JPEG**：Exif/XMP/Photoshop APP13/COM等を処理。EXIFのOrientation・解像度・許可された作成者等を小さなEXIF領域として再構成。XMPは指定されたテキストだけで最小限の文書を再作成。SOS以降のエントロピー符号化データはそのまま保持。EOI以降の余剰データは出力しません。
- **WebP**：RIFF EXIF/XMPチャンクを削除・置換し、VP8XのEXIF/XMP/ICCフラグとRIFFサイズを更新。VP8/VP8L・アニメーション画像データを保持。

画像の**ファイル構造**を検証しますが、画像デコーダを使ってピクセルを全展開し、完全にデコード可能かを保証するわけではありません。メタデータ削除後は同じ設定で再解析し、変更が再発しないことを検証します。

## 安全上の動作

- 上書き時は同じディレクトリの一時ファイルに全データを書き出し、検証後にファイルを置換します（Windowsでは `ReplaceFileW`、その他のOSではrename）。処理途中の書き込みで元画像を直接壊しません。
- コピー先のファイルやフォルダが既に存在する場合は上書きしません。
- PNG/JPEG/WebPとして認識できないファイル、構造が壊れたファイルは無理に編集しません。
- ファイル名の拡張子で対象を判断します（`.png`, `.jpg`, `.jpeg`, `.webp`、大文字も対応）。
- コピー先フォルダは作成済みかどうかを確認して新規作成します。1ファイル単位での安全性は確保しますが、フォルダ全体を1つのトランザクションとして扱いません。
- 出力ファイルには元の通常のファイルパーミッションを可能な範囲で引き継ぎますが、すべての拡張属性・ACL・ファイルシステム上のタイムスタンプを保持／消去するものではありません。

**置換の範囲について**：元画像にメタデータ領域がなければ置換値は追加しません。EXIFはIFD0のArtist・XPAuthor・Software・Copyrightと一部のExif SubIFDを対象とし、すべてのメーカー独自タグに対応するわけではありません。XMPの元データは細かく編集せず、置換指定されたcreator / copyright / software / commentsのみの最小XMPに再構成します。IPTC Photoshop領域は削除し、置換が指定されていれば最小XMPを代わりに作成できます。EXIFでUnicodeの作成者名はXPAuthorで保存しますが、ASCII限定のSoftware/CopyrightフィールドにはASCIIの名前のみ指定できます。AI生成情報等の任意文字列置換はPNGテキスト領域のみ対応します。これらは元のメタデータすべての完全な意味的編集を保証するものではありません。

既知のコンテナ以外に埋め込まれたプライバシー情報、サムネイルなどに画像として写り込んだ情報、カラープロファイル内部の記述文字列、クラウドサービスやOSが保持する作成記録は対象外です。

## 開発

依存クレートは使っていません。構成は `src/config.rs`（INI）、`src/ops.rs`（CLIのファイル操作）、`src/formats/`（PNG/JPEG/WebP）、`src/exif.rs`（EXIFフィールドの最小再構成）、`src/xmp.rs`（最小XMP生成）です。CIではUbuntu/Windowsで `cargo test --all-targets` とリリースビルドを実行します。
