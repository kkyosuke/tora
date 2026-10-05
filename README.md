# tora

<div align="center">

<pre>
  /\___/\     ┏┳┓ ┏━┓ ┏━┓ ┏━┓
 (=|• •|=)     ┃  ┃ ┃ ┣┳┛ ┣━┫
  (づ づ)      ╹  ┗━┛ ╹┗╸ ╹ ╹
</pre>

**いつものターミナルに、小さな便利を。**

AWS SSO・自己更新・シェル補完をひとつにまとめる Rust 製 CLI

[![CI](https://github.com/KKyosuke/tora/actions/workflows/ci.yml/badge.svg)](https://github.com/KKyosuke/tora/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg?logo=rust&logoColor=white)](https://rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-555555)](#インストール)

[インストール](#インストール) · [使い方](#使い方) · [AWS SSO](#aws-プロファイルと-sso) · [開発](#開発)

</div>

`tora` は、日々の作業で繰り返す操作を短いコマンドにまとめるツールです。
AWS プロファイルの選択から SSO ログイン、認証用シェルの起動までをつなぎます。
[usagi](https://github.com/KKyosuke/usagi) の配布・更新方式を参考に、便利なコマンドを少しずつ追加していきます。

## できること

| やりたいこと | コマンド | できること |
| --- | --- | --- |
| AWS の接続先を確認する | `tora aws profiles` | 設定済みプロファイルを一覧表示 |
| 接続先を選んでログインする | `tora aws sso` | 対話選択 → SSO ログイン → 認証用シェル |
| 最新版に更新する | `tora update` | 配布バイナリを検証して自己更新 |
| コマンド入力を楽にする | `tora completion zsh` | シェル補完スクリプトを生成 |

## 目次

- [インストール](#インストール)
- [使い方](#使い方)
- [AWS プロファイルと SSO](#aws-プロファイルと-sso)
- [開発](#開発)
- [リリース](#リリース)

## インストール

初回の GitHub Release 公開後、次のワンライナーでインストールできます。
Rust のインストールは不要です。Bash、curl、tar と SHA-256 計算ツール
（`sha256sum` または macOS 標準の `shasum`）を使用します。

```sh
curl -fsSL https://raw.githubusercontent.com/KKyosuke/tora/main/scripts/install.sh | bash
```

既定の配置先は `~/.tora/bin/tora` です。シェルの設定ファイルに追加してください。

```sh
export PATH="$HOME/.tora/bin:$PATH"
```

| OS | アーキテクチャ | 要件 |
| --- | --- | --- |
| macOS | Apple Silicon / Intel | — |
| Linux | x86_64 | glibc 2.35 以降 |

Windows、Linux ARM64、musl / Alpine は現時点では対象外です。

<details>
<summary>配置先・バージョンを指定する</summary>

配置先やインストールする版を指定する場合：

```sh
curl -fsSL https://raw.githubusercontent.com/KKyosuke/tora/main/scripts/install.sh | TORA_HOME="$HOME/.local/share/tora" bash -s -- --version v0.1.0
```

`TORA_HOME` を変更した場合は、実行時にも同じ値を設定し、`$TORA_HOME/bin` を PATH に追加してください。
`TORA_VERSION=v0.1.0 bash scripts/install.sh` による指定もできます。

</details>

## 使い方

```sh
tora --help
tora --version
tora update                       # 最新の安定版を導入
tora update --version v0.1.0       # 指定版を導入（再インストール・ダウングレードも可）
tora completion zsh               # 補完スクリプトを標準出力へ
```

`update` は `TORA_HOME/bin/tora`（既定は `~/.tora/bin/tora`）から実行してください。
ソースビルドや別の配置先のバイナリは自己更新を拒否します。
更新時は実行中のバイナリに埋め込んだインストーラーを使います。
環境変数 `TORA_VERSION` は `tora update` では無視し、指定には `--version` を使います。

<details>
<summary>更新の検証・ロックの仕組み</summary>

インストーラーは最新版のタグを一度だけ解決し、同じタグからアーカイブ、SHA-256、
バージョン情報を取得します。内容・チェックサム・実バイナリのバージョンを検証し、
同一ファイルシステム内の rename で置き換えます。検証失敗時は既存バイナリを保持します。
チェックサムは配布中の破損検知のためのもので、独立した署名ではありません。

同時更新は `$TORA_HOME/update.lock` で拒否します。通常の終了・シグナルでは自動解放します。
強制終了などでロックが残った場合は、他のインストーラーが動いていないことを確認してから
空の `update.lock` ディレクトリを `rmdir` で削除してください。

</details>

## AWS プロファイルと SSO

AWS CLI v2 を PATH に配置し、`aws configure sso` でプロファイルを設定してください。

```sh
tora aws profiles                  # 設定済みプロファイルの一覧
tora aws sso                       # 矢印キーで選択 → SSO ログイン → 認証用シェル
tora aws sso dev                   # プロファイルを指定して認証用シェルへ
aws sts get-caller-identity        # シェル内で認証先を確認
exit                              # 元のシェルに戻る
```

ログイン後は、Zsh / Bash のプロンプトに黄色の `[aws:プロファイル名]` が表示されます。
`exit` で元のシェルに戻ると、表示と環境も元に戻ります。

<details>
<summary>プロファイル選択・認証用シェルの動作</summary>

`dialoguer` による選択メニューは Enter で確定、Esc / q でキャンセルできます。
「Type a custom profile」で手入力でき、プロファイルがない場合も手入力できます。
標準入力がターミナルでない場合は番号選択になります（0 で手入力）。
プロファイルを引数に指定すると一覧取得・選択を省略します。

一覧取得には `aws configure list-profiles` を使うため、`~/.aws/config`、
`~/.aws/credentials` のほか `AWS_CONFIG_FILE` / `AWS_SHARED_CREDENTIALS_FILE` も
AWS CLI のルールに従って扱います。SSO ログインは `aws sso login --profile ...` を実行します。
[詳細は AWS 公式ドキュメント](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-sso.html)。

ログイン成功後、現在のターミナルで `$SHELL -i`（未設定なら `/bin/sh -i`）を起動し、
`AWS_PROFILE` を設定します。選択したプロファイルより優先される既存の
`AWS_ACCESS_KEY_ID`、`AWS_SECRET_ACCESS_KEY`、`AWS_SESSION_TOKEN`、
`AWS_SECURITY_TOKEN`、`AWS_DEFAULT_PROFILE` は子シェルから外します。
Zsh / Bash では、プロンプトの先頭に黄色の `[aws:プロファイル名]` を同じ行で表示します。
既存のプロンプトや設定ファイルを維持し、`exit` で元のシェルに戻ると表示も消えます。
それ以外のシェルではプロファイル設定のみ行います。
元のシェルの環境は変わりません。シェル設定ファイルで AWS 環境変数を再設定している場合は、
その設定が優先されるので `aws sts get-caller-identity` で確認してください。
認証用シェルを開くだけでは認証期限は延長されません。期限切れ時は再ログインしてください。

</details>

### ログイン方法を選ぶ

```sh
tora aws sso dev --no-shell                         # ログインだけ実行
tora aws sso dev --no-browser --use-device-code      # 別デバイスのブラウザーで認証
```

<details>
<summary>現在の Bash / Zsh に認証環境を反映する</summary>

現在の Bash / Zsh 自体に反映する場合は、ログイン成功時のみ出力するシェル文を評価できます。
認証メッセージ・選択メニューは標準エラーに出力します。

```sh
if tora_aws_env=$(tora aws sso dev --export); then
  eval "$tora_aws_env"
fi
unset tora_aws_env
```

`--export` はシェルを起動せず、上記のアクセスキー環境変数を解除して
`AWS_PROFILE` を設定する文を出力します。失敗時には何も出力しません。

</details>

## 開発

```sh
cargo run -- --help
cargo build
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s scripts/tests -v
```

- `src/cli.rs`: コマンドと引数の定義。便利コマンドは `Commands` に追加します。
- `src/main.rs`: コマンドの実行と終了コード。
- `src/aws.rs`: AWS CLI の呼び出し、プロファイル選択、認証用シェル。
- `tests/aws.rs`: AWS CLI・シェルを模擬する Rust 結合テスト。
- `src/update.rs`: 埋め込みインストーラーによる自己更新。
- `scripts/install.sh`: 初回インストールと更新で共用する処理。
- `scripts/tests/`: ネットワークを使わないインストール・失敗時の回帰テスト。

`Cargo.lock` をコミットして依存を固定します。Python テストの一時ファイルは
リポジトリ内の `.cache` に作成します。

## リリース

1. `Cargo.toml` の `version` を更新し、`cargo check` で `Cargo.lock` を更新します。
2. CI が成功した変更を main に反映します。
3. 同じバージョンのタグを push します。

```sh
git tag v0.1.0
git push origin v0.1.0
```

`.github/workflows/release.yml` がタグと Cargo のバージョンの一致、テスト、lint を確認し、
以下のアセットを生成します（各アーカイブに `.sha256` と `.version` が付属）。

- `tora-macos-arm64.tar.gz`
- `tora-macos-amd64.tar.gz`
- `tora-linux-amd64.tar.gz`

全ビルド完了後、すべてのアセットを draft release にアップロードしてから公開します。
GitHub の標準リリースノートを自動生成します。`vMAJOR.MINOR.PATCH` の安定版のみ対応します。
公開済みタグ・アセットは変更せず、修正は新しいバージョンとして出してください。
公開処理に失敗して draft が残った場合は、アセットを確認して公開するか、draft を削除して
publish job を再実行します。

GitHub Actions と `contents: write` 権限が必要です。公開ワンライナーの利用には
リポジトリと Release が公開されている必要があります。この基盤の追加だけではリリースは公開されません。
