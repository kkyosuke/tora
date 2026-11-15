# tora

<div align="center">

<pre>
 /\___/\    ___  ___  __   _&nbsp;
(=|o o|=)    |  |   | |_) |_|
 (")_(")     |  |___| | \ | |
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
| AWS の接続先を指定して実行する | `tora aws exec dev -- terraform plan` | 選択したプロファイルの環境でコマンドを実行 |
| 環境の問題を調べる | `tora doctor` | インストール・補完・AWS 認証を診断 |
| AWS の実際の接続先を確認する | `tora aws whoami` | アカウント・ARN・リージョンを表示 |
| SSO セッションを終了する | `tora aws logout` | 全プロファイルの SSO キャッシュをログアウト |
| 既定値を保存する | `tora config set aws.profile dev` | プロファイル・リージョンを設定 |
| 更新があるか確認する | `tora update --check` | バイナリを更新せずバージョンを比較 |
| 最新版に更新する | `tora update` | 配布バイナリを検証して自己更新 |
| コマンド入力を楽にする | `tora completion install` | シェルを判定して補完を設定 |

## 目次

- [インストール](#インストール)
- [使い方](#使い方)
- [環境診断と設定](#環境診断と設定)
- [シェル補完](#シェル補完)
- [AWS プロファイルと SSO](#aws-プロファイルと-sso)
- [プロファイルを指定してコマンドを実行](#プロファイルを指定してコマンドを実行)
- [開発](#開発)
- [リリース](#リリース)

## インストール

次のワンライナーで公開済みの最新リリースをインストールできます。
Rust のインストールは不要です。Bash、curl、tar と SHA-256 計算ツール
（`sha256sum` または macOS 標準の `shasum`）を使用します。

```sh
curl -fsSL https://raw.githubusercontent.com/KKyosuke/tora/main/scripts/install.sh | bash
```

既定の配置先は `~/.tora/bin/tora` です。インストール時に `$SHELL` を判定し、
Zsh は `${ZDOTDIR:-$HOME}/.zshrc`、Bash は `~/.bashrc` とログイン用設定ファイル
（既存の `.bash_profile` → `.bash_login` → `.profile` の順、なければ `.bash_profile`）へ
PATH と `TORA_HOME` を自動設定します。再実行時は管理ブロックを更新するため重複しません。
既存の設定とシンボリックリンクは維持します。

新しいターミナルを開くと `tora` が使えます。今のシェルですぐ使う場合は、
インストール完了時に表示されるコマンドを実行してください（既定の配置先なら以下）。

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

`TORA_HOME` を変更した場合も、その配置先をシェル設定へ保存します。
Bash/Zsh 以外、`HOME` 未設定、書き込み不可、管理ブロックの破損などで自動設定できない場合は、
バイナリのインストールを完了したうえで警告と PATH の設定先を表示します。
その場合は実行時にも同じ `TORA_HOME` を設定し、`$TORA_HOME/bin` を PATH に追加してください。
PATH で扱えないコロン・改行を含む配置先は使用できません。
`TORA_VERSION=v0.1.0 bash scripts/install.sh` による指定もできます。

</details>

## 使い方

```sh
tora --help
tora --version
tora update --check               # 更新せずに現在と最新版を比較
tora update                       # 最新の安定版を導入
tora update --version v0.1.0       # 指定版を導入（再インストール・ダウングレードも可）
tora completion zsh               # 補完スクリプトを標準出力へ
```

`update` は `TORA_HOME/bin/tora`（既定は `~/.tora/bin/tora`）から実行してください。
ソースビルドや別の配置先のバイナリは自己更新を拒否します。
更新時は実行中のバイナリに埋め込んだインストーラーを使います。
`update --check` はソースビルドからも利用でき、インストール先の作成・ロック取得・
バイナリのダウンロードを行いません。最新版への HTTPS リクエストには `curl` を使います。
最新版と同じ場合・手元の方が新しい場合も区別して表示します。通信エラーや
未公開リリースは終了コード1、確認成功は更新の有無にかかわらず0です。
`--check` と `--version` は併用できません。

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

## 環境診断と設定

```sh
tora doctor                       # 複数の問題と修復手順をまとめて表示
tora doctor --profile dev         # 指定プロファイルの認証を確認
tora config                       # 保存先と設定済みの既定値を表示
tora config set aws.profile dev
tora config set aws.region ap-northeast-1
tora config get aws.profile
tora config unset aws.region
```

`doctor` は実行中のバイナリの配置、PATH の優先順、更新に必要なツール、
Bash/Zsh の補完設定、AWS CLI v2、STS による実際の認証状態を確認します。
問題があっても残りの診断を続け、すべて正常なら終了コード0、要対応なら1を返します。
自動修復・SSO ログインは行いません。AWS CLI 自身の認証情報の更新は通常どおり行われます。
補完は `tora completion install` が保存したカスタムパスにも対応します。
親シェルが補完を読み込み済みか、手動の補完設定が有効かは判定できません。

設定ファイルは `$TORA_HOME/config`（既定 `~/.tora/config`）です。
`aws.profile` と `aws.region` だけを保存し、AWS の設定ファイルや認証情報は変更しません。
`config list` は `config` と同じ動作、`config get` は未設定なら終了コード1です。
書き込みは同じディレクトリの一時ファイルから置き換え、同時書き込みは
`config.lock` ディレクトリで拒否します。強制終了でロックが残った場合は、
他の設定変更が動作していないことを確認して空のディレクトリを削除してください。

プロファイルの優先順位は、明示した引数 → `AWS_PROFILE` → `AWS_DEFAULT_PROFILE` →
`aws.profile` の保存値です。SSO と exec はすべて未設定なら選択メニューを表示します。
`aws sso --select` は環境変数と保存値を無視して選び直します。
リージョンは `AWS_REGION` → `AWS_DEFAULT_REGION` → `aws.region` の保存値 →
AWS CLI 側の設定の順です。選んだリージョンは子プロセス・認証用シェル・
`--export` の出力にも反映します。親シェルの環境は変更しません。

### AWS の接続先確認とログアウト

```sh
tora aws whoami                   # 現在の認証情報のアカウント・ARN・ユーザーID・リージョン
tora aws whoami dev               # 指定プロファイルで確認
tora aws logout                   # 全プロファイルの SSO セッションを終了
```

`whoami` は対話選択や自動ログインを行いません。引数なしでは現在の環境変数の
アクセスキーを維持して AWS の認証チェーンを使うため、表示されるプロファイル設定より
環境変数の認証情報が優先される場合があります。実際の接続先はアカウントと ARN で確認してください。
引数でプロファイルを指定した場合は、SSO / exec と同様に競合するアクセスキー環境変数を外します。
未設定のリージョンは `(not configured)` と表示します。

`logout` は `aws sso logout` を呼びます。対象は特定のプロファイルだけでなく、
**全プロファイルの SSO トークンと SSO 由来の一時認証情報のキャッシュ**です。
親シェルの環境変数は変更しません。
[AWS CLI の logout 仕様](https://docs.aws.amazon.com/cli/latest/reference/sso/logout.html)。

## シェル補完

```sh
tora completion install
```

親プロセス名から Zsh / Bash を判定し、判定できなければ `$SHELL` を参照します。
どちらでも判定できない場合は選択メニューを表示します。ラッパーや別のシェル経由で
起動する場合は `--shell zsh` / `--shell bash` で対象を明示できます。

1. 既存の tora 補完設定を確認します。
2. 未設定なら保存先を尋ねます。Enter で既定の `$TORA_HOME/completions/zsh` または
   `bash`（`TORA_HOME` 未設定時は `~/.tora/completions/` 配下）を選べます。
   保存先はファイルのパスです。相対パスと `~/` も指定できます。
3. 変更対象のファイルを表示し、確認後に補完の読み込み設定を追加・更新します。
   確認の既定値は「いいえ」で、キャンセル時はファイルを変更しません。

| シェル | 読み込み設定を追加するファイル |
| --- | --- |
| Zsh | `$ZDOTDIR/.zshrc`（環境変数 `ZDOTDIR` 未設定時は `~/.zshrc`） |
| Bash | `~/.bashrc` と、ログイン時に使われる設定ファイル |

Bash のログイン用設定は `.bash_profile` → `.bash_login` → `.profile` の順で
最初に存在するファイルを使い、どれもなければ `.bash_profile` を作成します。
二重に読み込まれても、同じシェルでは一度だけ補完を登録します。
Zsh の `ZDOTDIR` を使う場合は、実行前に環境変数として export してください。

新しいシェルを開くと `tora ` や `tora aws ` の後に Tab で候補を補完できます。
Zsh では説明も表示できます。現在のシェルで有効にするコマンドは完了時に表示します。
補完はシェル起動時に PATH 上の `tora` から生成するため、更新後の新しいシェルにも追従します。
設定ファイルの途中で `return` / `exit` する場合は、追加した管理ブロックをその前に移してください。
AWS プロファイル名の動的な Tab 補完には対応していません。

再実行時は保存済みのパスを使い、同じ内容なら何も変更しません。
変更があれば `# >>> tora completion >>>` から `# <<< tora completion <<<` までを更新し、
それ以外の内容と既存ファイルの権限、設定ファイルのシンボリックリンクを保持します。
手書きの `source <(tora completion zsh)` などを検出した場合は、重複を避けるため停止します。
その設定を使い続けるか、手動で削除してから実行してください。確認対象は上記の設定ファイルと
保存先です。プラグインや別ファイルから読み込まれた設定を網羅的に検出するものではありません。

```sh
tora completion install --shell zsh --path ~/.config/tora/completion.zsh
# 自動化用。対象シェルを指定し、表示される変更を承認する場合のみ使用
tora completion install --shell bash --yes
# 既存の補完スクリプト出力も利用可能（fish / powershell / elvish にも対応）
tora completion zsh
```

`--path` で移動した場合は管理ブロックの参照先を更新し、古いファイルは残します。
非対話実行では `--shell` と `--yes` が必要です。

## AWS プロファイルと SSO

AWS CLI v2 を PATH に配置し、`aws configure sso` でプロファイルを設定してください。

```sh
tora aws profiles                  # 設定済みプロファイルの一覧
tora aws sso                       # 現在のプロファイルで認証確認 → 必要ならログイン → シェル
tora aws sso --select              # プロファイルを選び直して認証用シェルへ
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
「Search profiles」で名前の部分一致検索ができます（大文字・小文字は区別しません）。
検索語を空にすると一覧全体に戻ります。検索後も手入力・キャンセルできます。
標準入力がターミナルでない場合は番号選択になります（0 で手入力）。
番号選択では `/dev` のように入力して検索、`/` で検索解除できます。
プロファイルは引数、空でない `AWS_PROFILE`、空でない `AWS_DEFAULT_PROFILE` の順で決定し、
`aws.profile` の保存値を続いて参照し、すべて未設定なら選択メニューを表示します。`--select` は環境変数と保存値を無視して
選択メニューを表示します（プロファイル引数との併用は不可）。

SSO の認証確認またはログイン成功時と `aws exec` の実行直前に、選択したプロファイル名を
`$TORA_HOME/aws-profile-history`（既定は `~/.tora/aws-profile-history`）へ保存します。
直近20件を保持し、選択メニューでは最近使った設定済みプロファイルを上に表示します。
履歴は SSO と exec で共通です。設定から削除した名前は一覧に追加しません。
`aws profiles` の出力順は変わりません。履歴に認証情報は保存しません。
履歴の読み取りに失敗した場合は通常の一覧順で表示し、保存に失敗しても処理を続けます。
履歴ファイルを削除すると履歴をリセットできます。

一覧取得には `aws configure list-profiles` を使うため、`~/.aws/config`、
`~/.aws/credentials` のほか `AWS_CONFIG_FILE` / `AWS_SHARED_CREDENTIALS_FILE` も
AWS CLI のルールに従って扱います。認証確認には `aws sts get-caller-identity --profile ...` を使い、
成功すればブラウザーを開かずに進みます。AWS CLI が更新可能な認証情報は自動更新します。
未ログイン・期限切れなど既知の認証エラーの場合だけ、
`aws sso login --profile ...` を実行します。通信障害・設定不備など、
その他のエラーは表示して停止します。確認・ログイン時も後述の競合する環境変数を外します。
[詳細は AWS 公式ドキュメント](https://docs.aws.amazon.com/cli/latest/userguide/cli-configure-sso.html)。

認証確認またはログイン成功後、現在のターミナルで `$SHELL -i`（未設定なら `/bin/sh -i`）を起動し、
`AWS_PROFILE` を設定します。選択したプロファイルより優先される既存の
`AWS_ACCESS_KEY_ID`、`AWS_SECRET_ACCESS_KEY`、`AWS_SESSION_TOKEN`、
`AWS_SECURITY_TOKEN`、`AWS_DEFAULT_PROFILE` は子シェルから外します。
Zsh / Bash では、プロンプトの先頭に黄色の `[aws:プロファイル名]` を同じ行で表示します。
既存のプロンプトや設定ファイルを維持し、`exit` で元のシェルに戻ると表示も消えます。
それ以外のシェルではプロファイル設定のみ行います。
元のシェルの環境は変わりません。シェル設定ファイルで AWS 環境変数を再設定している場合は、
その設定が優先されるので `aws sts get-caller-identity` で確認してください。
認証用シェルを開くだけでは認証期限は延長されません。期限切れ時は `tora aws sso` を再実行してください。

</details>

### ログイン方法を選ぶ

```sh
tora aws sso dev --no-shell                         # 認証確認・必要なログインのみ実行
tora aws sso dev --no-browser --use-device-code      # 別デバイスのブラウザーで認証
```

<details>
<summary>現在の Bash / Zsh に認証環境を反映する</summary>

現在の Bash / Zsh 自体に反映する場合は、認証成功時のみ出力するシェル文を評価できます。
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

## プロファイルを指定してコマンドを実行

```sh
tora aws exec dev -- terraform plan
tora aws exec dev -- aws sts get-caller-identity
tora aws exec -- terraform plan    # プロファイルを検索・選択して実行
```

`--` 以降をコマンドと引数として、そのまま実行します。シェルによる追加の解釈は行いません。
子プロセスに `AWS_PROFILE` を設定し、認証用シェルと同じアクセスキー環境変数を解除します。
元のシェルの環境は変わりません。標準入出力・標準エラー出力を引き継ぎ、
対応プラットフォーム（macOS / Linux）ではプロセスを置き換えるため、終了コードと
シグナル終了もそのまま呼び出し元に伝わります。

既存の認証情報を使い、自動で SSO ログインは行いません。
必要なら先に `tora aws sso dev --no-shell` を実行してください。
パイプ入力を子コマンドへ渡す場合は、プロファイルを明示して選択操作を省略してください。

## 開発

リポジトリ内での `cargo run` / `cargo run --release` / `cargo test` は
Cargo runner (`scripts/run-dev.sh`) を通し、本番のインストール環境から分離します。
最適化の有無では区別しないため、`--release` でも開発用の保存先になります。

| 対象 | Cargo 経由の開発実行 | インストール済み tora |
| --- | --- | --- |
| 設定・履歴・補完ファイル | `<repo>/.cache/dev/tora/` | `$TORA_HOME`（既定 `~/.tora/`） |
| シェル起動設定 | `<repo>/.cache/dev/home/` | 通常の `HOME` / `ZDOTDIR` |
| AWS 設定・SSO キャッシュ | `<repo>/.cache/dev/home/.aws/` | 通常の AWS 設定先 |

インストーラーが export した `TORA_HOME` も、Cargo 実行時には開発用へ置き換えます。
`HOME`、`ZDOTDIR`、一時ファイル・XDG の保存先も `.cache/dev/` 配下になります。
本番の AWS プロファイル・リージョン・アクセスキー・設定ファイル指定などの `AWS_*` 環境変数は
引き継がず、インスタンスメタデータ経由の認証も無効にします。
開発用の AWS 設定と SSO ログインは別途必要です。例えば：

```sh
./scripts/run-dev.sh "$(command -v aws)" configure sso
cargo run -- aws sso dev
cargo run -- config set aws.profile dev
cargo run -- completion install --shell zsh --yes
```

子プロセスから `tora` を呼ぶ場合も、その Cargo 実行で使った開発バイナリを優先します。
`cargo run -- update` は既存の配置チェックで拒否され、本番バイナリを置き換えません。
`cargo run -- update --check` によるバージョン確認は利用できます。

この分離は **Cargo runner を通る実行**が対象です。`target/debug/tora` などの直接実行は
通常の環境を使うため、開発時は `cargo run` または `./scripts/run-dev.sh target/debug/tora ...` を使ってください。
リポジトリ外から `--manifest-path` だけで起動すると Cargo がこの設定を読まない場合があります。
その場合は `--config /path/to/tora/.cargo/config.toml` も指定してください。
任意の外部コマンドや、明示的に指定した絶対パスへのアクセスを制限するサンドボックスではありません。
配布バイナリと通常の `tora` の保存先・動作は変更しません。

```sh
cargo run -- --help
cargo build
bash scripts/check.sh              # CI・リリースと共通の検証
```

- `src/cli.rs`: コマンドと引数の定義。便利コマンドは `Commands` に追加します。
- `src/main.rs`: コマンドの実行と終了コード。
- `src/paths.rs`: 保存先とインストール済みバイナリの判定を共通化。
- `src/aws.rs`: AWS CLI の呼び出し、プロファイル選択、認証用シェル。
- `src/aws/settings.rs`: 1操作ごとの設定の読み込み、認証環境と export の生成。
- `tests/aws.rs`: AWS CLI・シェルを模擬する Rust 結合テスト。
- `src/config.rs` / `src/doctor.rs`: 既定値の保存と環境診断。
- `tests/helpers.rs`: 設定・接続先確認・ログアウト・診断・更新確認の結合テスト。
- `src/update.rs`: 埋め込みインストーラーによる自己更新。
- `scripts/install.sh`: 初回インストールと更新で共用する処理。
- `scripts/tests/`: ネットワークを使わないインストール・失敗時の回帰テスト。

`Cargo.lock` をコミットして依存を固定します。Python テストの一時ファイルは
リポジトリ内の `.cache` に作成します。

## リリース

最新の main と同じコミットの、未コミット変更がないチェックアウトで実行します。
Python 3、Rust/Cargo、Git、認証済みの GitHub CLI (`gh auth login`) と
リポジトリへの push・PR 作成権限が必要です。

```sh
python3 scripts/release.py
```

`major` / `minor` / `patch` を次のバージョン表示から選ぶと、
`Cargo.toml`・`Cargo.lock` を更新し、`release/vX.Y.Z` ブランチにコミット・push して
main 向けの PR を作成します。種類を直接指定するワンライナーも使えます。

```sh
python3 scripts/release.py patch
```

既存のリリースブランチやタグがある場合は上書きせず停止します。
途中で失敗した場合は変更とブランチを保持します。`git status` を確認して復旧してください。
push 済みで PR 作成だけ失敗した場合は、そのブランチで `gh pr create --base main` を実行できます。

PR の CI 成功後に main にマージし、最新の main を取得したチェックアウトで
同じバージョンのタグを push します（以下は `0.1.0` の例）。
PR 作成やマージだけでは公開されません。

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
リポジトリと Release が公開されている必要があります。
