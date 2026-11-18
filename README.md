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
| 今のシェルの接続先を切り替える | `tora-use dev` | 必要なら SSO ログインし、今のシェルの `AWS_PROFILE` を切り替え（要 `tora completion install`） |
| 今の認証状態を確認する | `tora aws status` | `whoami` の内容に SSO 期限の残り時間を加えて表示 |
| AWS の接続先を指定して実行する | `tora aws exec dev -- terraform plan` | 選択したプロファイルの環境でコマンドを実行 |
| 必要なときだけログインして実行する | `tora aws exec --login dev -- terraform plan` | 認証を確認し、期限切れなら SSO ログインしてから実行 |
| EC2 にログインする | `tora aws ssm` | インスタンスを選んで Session Manager で接続（前回の接続先を表示） |
| RDS などにローカルから接続する | `tora aws tunnel dev` | 踏み台経由でローカルポートを転送（前回の転送先を再利用） |
| 環境の問題を調べる | `tora doctor` | インストール・補完・AWS 認証・SSO 期限を診断 |
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
- [Session Manager でインスタンスに接続](#session-manager-でインスタンスに接続)
- [踏み台経由でポートを転送](#踏み台経由でポートを転送)
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
`--profile` または既定のプロファイル（環境変数・`aws.profile`）があれば、認証の結果に
`tora aws status` と同じ SSO トークンの期限と残り時間を加えます。期限切れは認証の項目 1 件として
要対応になり、判定できない場合は `unknown` と表示して失敗にはしません。
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

Zsh / Bash では、プロファイルを指定する位置で Tab を押すと
`aws configure list-profiles` で得られるプロファイル名を補完します。対象は
`tora aws sso` / `tora aws exec` / `tora aws whoami` / `tora aws status` のプロファイル引数、
`tora doctor --profile`、`tora config set aws.profile` の値です。`tora aws exec` の
`--` より後（実行するコマンド）では補完しません。AWS CLI が見つからない・失敗した場合は
エラーを表示せず、プロファイル名の候補を出さないだけです。Tab のたびに AWS CLI を呼ぶため、
候補の表示までに少し時間がかかることがあります。
設定済みの場合も `tora completion install` の再実行は不要で、tora の更新後に新しいシェルを開けば有効になります。
ただし読み込み設定（保存先のファイル）自体が変わる更新、たとえば
[`tora-use`](#今のシェルのプロファイルを切り替える) の追加では再実行が必要です。`tora doctor` が知らせます。
fish / powershell / elvish はプロファイル名の補完に対応していません。

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
tora aws status                    # プロファイル・認証先・SSO 期限の残り時間を確認
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
直前に使ったプロファイルには `(last used)`、それ以前に使ったものには `(recent)` を付けます。
履歴は SSO・exec・ssm・tunnel で共通です。設定から削除した名前は一覧に追加しません。
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
認証用シェルを開くだけでは認証期限は延長されません。残り時間は `tora aws status` で確認し、
期限切れ時は `tora aws sso` を再実行してください。

</details>

### 認証状態を確認する

```sh
tora aws status                    # 現在のプロファイルの認証状態
tora aws status dev                # プロファイルを指定
```

```text
Profile: dev
Account: 123456789012
ARN: arn:aws:sts::123456789012:assumed-role/AWSReservedSSO_Admin_0123456789abcdef/alice
User ID: AROAEXAMPLE:alice
Region: ap-northeast-1
SSO expires: 2026-11-16T09:30:00Z (3h 12m remaining)
```

`whoami` との違い: `status` は常にプロファイルを決めて（未設定なら選択メニュー）その認証で確認し、SSO トークンの期限と残り時間を追加で表示します。

プロファイルは `tora aws sso` と同じ規則（引数、空でない `AWS_PROFILE`、空でない
`AWS_DEFAULT_PROFILE`、`aws.profile` の保存値、いずれもなければ選択メニュー）で決定します。
未ログイン・期限切れの場合は `tora aws sso` を案内して終了コード 1 で終了します。
自動でログインはしません。

<details>
<summary>認証状態の確認方法と SSO 期限の求め方</summary>

認証先の確認と表示は `tora aws whoami <profile>` と同じ処理です。確認時は `tora aws sso` と同じく、
競合するアクセスキー環境変数を外します。通信障害・設定不備など、
未ログイン・期限切れ以外のエラーは表示して停止します。

SSO 期限は `aws configure get` で `sso_session`（なければ `sso_start_url`）を取得し、
`~/.aws/sso/cache/` にある対応するキャッシュファイルの `expiresAt` から求めます。
`sso-session` を使う設定と、プロファイルに `sso_start_url` を直接書く従来の設定の
両方に対応します。時刻はキャッシュの記録どおり（UTC）に表示します。
SSO を使わないプロファイルやキャッシュが見つからない場合など、判定できなければ
`unknown` と表示し、失敗にはしません。キャッシュからは期限だけを読み取り、
トークンの値は表示・出力しません。

`sso-session` を使う設定では、表示される期限は現在のアクセストークンのものです。
SSO セッションが続く間は AWS CLI が自動更新するため、
`refreshed automatically while the SSO session lasts` と表示します。
SSO トークンの期限後も、取得済みのロール認証情報が有効な間は認証に成功し、
期限は `(expired)` と表示されます。

</details>

### ログイン方法を選ぶ

```sh
tora aws sso dev --no-shell                         # 認証確認・必要なログインのみ実行
tora aws sso dev --no-browser --use-device-code      # 別デバイスのブラウザーで認証
```

### 今のシェルのプロファイルを切り替える

`tora completion install` で設定した Bash / Zsh では、シェル関数 `tora-use` が使えます。
子プロセスである `tora` は親シェルの環境を変えられないため、`tora aws sso --export` の
出力をシェル関数が評価して、今のシェルの `AWS_PROFILE` を切り替えます。

```sh
tora-use dev                    # 必要な場合だけ SSO ログインし、AWS_PROFILE=dev に切り替え
tora-use                        # プロファイルを検索・選択（AWS_PROFILE や保存値があっても選択）
tora-use --no-browser dev       # --no-browser / --use-device-code も指定可能
```

- ログインの失敗・選択のキャンセル時は環境を変えず、0 以外で終了します。
  `set -e` / `set -u` を有効にしたシェルでも使えます。
- 成功時は `tora aws sso --export` と同じく、アクセスキー環境変数を解除して `AWS_PROFILE`
  （リージョンを設定済みなら `AWS_REGION` / `AWS_DEFAULT_REGION`）を設定します。
- プロファイル名は Tab で補完できます。
- 同名のエイリアス・コマンドがあれば定義しません。`sh`（POSIX モードの Bash）でも定義しません。
- 既存の補完設定は、`tora completion install` を再実行すると `tora-use` を含む内容に更新されます。
  更新が必要な場合は `tora doctor` が知らせます。

<details>
<summary>シェル関数を使わずに現在の Bash / Zsh へ反映する</summary>

認証成功時のみ出力するシェル文を評価できます。認証メッセージ・選択メニューは標準エラーに出力します。

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
tora aws exec --login dev -- terraform plan    # 必要な場合だけ SSO ログインしてから実行
```

プロファイルを省略すると、`AWS_PROFILE` → `AWS_DEFAULT_PROFILE` → `aws.profile` の保存値 →
選択メニューの順で決まります（[環境診断と設定](#環境診断と設定)）。`--login` でも同じです。

`--` 以降をコマンドと引数として、そのまま実行します。シェルによる追加の解釈は行いません。
子プロセスに `AWS_PROFILE` を設定し、認証用シェルと同じアクセスキー環境変数を解除します。
元のシェルの環境は変わりません。標準入出力・標準エラー出力を引き継ぎ、
対応プラットフォーム（macOS / Linux）ではプロセスを置き換えるため、終了コードと
シグナル終了もそのまま呼び出し元に伝わります。

既定では既存の認証情報を使い、自動で SSO ログインは行いません。
`--login` を付けると、実行前に `tora aws sso` と同じ方法で認証を確認します。
有効ならそのまま実行し、未ログイン・期限切れなど既知の認証エラーの場合だけ
`aws sso login` を実行します。その他のエラーやログイン失敗時は、コマンドを実行せずに
0 以外で終了します。認証確認・ログインのメッセージは標準エラー出力へ書き出すため、
子コマンドの標準出力はそのままパイプに渡せます。

`--login`、`--no-browser`、`--use-device-code` は `--` より前に指定します
（`--` 以降は子コマンドの引数です）。`--no-browser` / `--use-device-code` には
`--login` が必要です。

```sh
tora aws exec --login dev -- aws s3 ls | sort
tora aws exec --login --no-browser dev -- terraform plan
```

パイプ入力を子コマンドへ渡す場合は、プロファイルを明示して選択操作を省略してください。

## Session Manager でインスタンスに接続

```sh
tora aws ssm                              # プロファイル → インスタンスを選んで接続
tora aws ssm dev                          # プロファイルを指定してインスタンスを選択
tora aws ssm dev -t i-0123456789abcdef0   # 接続先を指定（一覧取得を省略）
```

```text
? Instance (Esc/q to cancel) ›
❯ batch-worker  i-0bbb222  Online          Amazon Linux  (last used)
  web-1         i-0aaa111  Online          Amazon Linux  (recent)
  bastion       i-0ccc333  ConnectionLost  Ubuntu
  Search instances
```

AWS CLI v2 に加えて [Session Manager プラグイン](https://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html)
（`session-manager-plugin`）が必要です。見つからない場合はログイン前に停止します。
プロファイルの決定と認証は `tora aws sso` と同じです。認証情報が有効ならログインを省略し、
必要な場合だけ SSO ログインします（`--select` / `--no-browser` / `--use-device-code` も同様）。
インスタンス ID はプロファイル引数ではなく `-t` / `--target` で指定します。

一覧には `aws ssm describe-instance-information` で取得した、Session Manager 管理下の
インスタンスを表示します。EC2 の `Name` タグは `aws ec2 describe-instances` で補い、
取得できない場合はホスト名を表示します。Online のものを先にして名前順（大文字・小文字を区別せず、名前なしは最後）に並べます。
「Search instances」で名前・ID・プラットフォームを部分一致検索できます。
標準入力がターミナルでない場合は番号選択（`/text` で検索、`q` でキャンセル）になります。

接続先は `$TORA_HOME/aws-ssm-history` に、プロファイルごとに直近20件まで保存します。
次回は Online のものと Online 以外のものそれぞれで、最近使ったものを上に表示します。
直前の接続先に `(last used)`、それ以前の接続先に `(recent)` を付けるため、
直前の接続先が Online なら Enter だけで再接続できます。
直前の接続先がすでに存在しない場合は、`(last used)` を付けません。
プロファイルと接続先の履歴は、セッションを開始する直前に保存します（選択をキャンセルした場合は保存しません）。

接続時は tora のプロセスを `aws ssm start-session --target ...` に置き換えるため、
終了コードやシグナルもそのまま伝わります。番号選択で標準入力から読み込んだ残りの入力は
セッションに渡らないため、セッションへ入力を流し込む場合は `-t` で接続先を指定してください。

## 踏み台経由でポートを転送

```sh
tora aws tunnel dev -t i-0123456789abcdef0 -r mydb.xxxx.ap-northeast-1.rds.amazonaws.com:5432
tora aws tunnel dev                       # 2回目以降: 履歴から転送先を選ぶだけ
tora aws tunnel dev -r db.internal:3306 -l 13306   # ローカルポートを指定
```

```text
? Port forwarding (Esc/q to cancel) ›
❯ mydb.xxxx.ap-northeast-1.rds.amazonaws.com:5432  via i-0123456789abcdef0  (last used)
  cache.internal:6379  via i-0123456789abcdef0  (recent)
  Enter a new remote host
Forwarding localhost:5432 -> mydb.xxxx.ap-northeast-1.rds.amazonaws.com:5432 via i-0123456789abcdef0 with profile "dev".
Connect to localhost:5432 while the session runs; press Ctrl+C to stop.
```

Session Manager のポート転送（`AWS-StartPortForwardingSessionToRemoteHost`）で、
踏み台インスタンスから到達できるホスト（RDS など）へローカルポートを転送します。
プロファイルの決定・認証・Session Manager プラグインの確認・インスタンスの選択は
`tora aws ssm` と同じです（`--select` / `--no-browser` / `--use-device-code` も同様）。

- `-r` / `--remote HOST:PORT`: 転送先。ホスト名か IPv4 アドレスとポート（1-65535）を指定します。
  `-` で始まる値、空白・制御文字などを含む値は AWS を呼ぶ前に拒否します。
- `-t` / `--target ID`: 踏み台インスタンス。省略時は、同じプロファイルで同じ `--remote` を
  使ったことがあれば直前の踏み台を使い（その旨を表示）、なければインスタンス一覧から選びます。
- `-l` / `--local PORT`: ローカルポート。省略時はリモートと同じポートを使い、
  使用中なら空いているポートを選びます。指定したポートが使用中ならログイン前に停止します。
  どちらの場合も接続先の `localhost:<port>` を表示します。

`--remote` を省略すると、そのプロファイルで使った転送先（踏み台と接続先の組）を新しい順に表示し、
選ぶだけで前回と同じ転送を開始します。「Enter a new remote host」または履歴がない場合は
接続先を入力し、続けてインスタンスを選びます。`-t` を指定した場合は、そのインスタンス経由の履歴だけを表示します。
標準入力がターミナルでない場合は番号選択（`0` で新規入力、`q` でキャンセル）になります。

転送先は `$TORA_HOME/aws-tunnel-history` に、プロファイルごとに直近20件まで保存します。
使った踏み台は `tora aws ssm` の接続先履歴にも記録します。履歴は転送を開始する直前に保存し、
選択をキャンセルした場合は保存しません。
接続時は tora のプロセスを `aws ssm start-session` に置き換えるため、Ctrl+C で転送を終了でき、
終了コードもそのまま伝わります。

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
- `src/aws/status.rs`: `whoami` の表示に SSO 期限を加える `aws status`。
- `tests/aws.rs`: AWS CLI・シェルを模擬する Rust 結合テスト。
- `src/config.rs` / `src/doctor.rs`: 既定値の保存と環境診断。
- `tests/helpers.rs`: 設定・接続先確認・ログアウト・診断・更新確認の結合テスト。
- `src/update.rs`: 埋め込みインストーラーによる自己更新。
- `src/completion.rs`: 補完スクリプトの生成・インストールと、プロファイル名の動的補完。
- `tests/completion.rs`: 補完のインストールと実際の Bash での補完の結合テスト。
- `scripts/install.sh`: 初回インストールと更新で共用する処理。
- `scripts/tests/`: ネットワークを使わないインストール・失敗時の回帰テスト。

`Cargo.lock` をコミットして依存を固定します。Python テストの一時ファイルは
リポジトリ内の `.cache` に作成します。

## リリース

GitHub の **Actions → Release PR → Run workflow** でブランチに `main`、
`bump` に `major` / `minor` / `patch` を選ぶと、次の流れでリリースします。

1. 最新 main の `Cargo.toml`・`Cargo.lock` を更新し、共通テスト・lint を実行
2. `release/vX.Y.Z` の PR を作り、squash auto-merge を有効化
3. PR の必須 CI が成功すると自動マージ
4. main のバージョン増加を Auto Release が検知し、同じコミットへタグを作成
5. 3環境のビルド・検証が成功すると GitHub Release を公開

```sh
gh workflow run release-pr.yml --ref main -f bump=patch
```

usagi と同じく、Actions secret **`RELEASE_PR_TOKEN`** が必要です。
このリポジトリの **Contents: Read and write** と **Pull requests: Read and write** を持つ
専用の fine-grained PAT を登録してください。トークンの利用者にもリポジトリへの書き込み権限が必要です。
未登録なら PR やブランチを作る前に停止します。リポジトリの auto-merge を有効にし、
main の必須チェックとして `test` を設定してください（tora では設定済み）。

専用トークンで PR 作成とマージ後の push イベントを発生させ、CI の手動承認を不要にします。
標準の `GITHUB_TOKEN` で作ったタグに別ワークフローの起動を依存させず、
Auto Release から reusable な Release ワークフローを直接呼び出します。
[GitHub のトークンとワークフロー起動の仕様](https://docs.github.com/en/actions/concepts/security/github_token)。

PR 作成ワークフローの Summary に PR リンクを表示します。既存の同名ブランチは上書きしません。
手動でマージしたバージョン更新 PR も自動リリースの対象です。バージョンが同じ場合は公開せず、
引き下げ・Cargo.lock との不一致はエラーになります。

ローカルからは `python3 scripts/release.py patch --check --auto-merge` で同じ PR を作成できます。
`--auto-merge` を省略すると PR 作成までで停止しますが、マージ後のリリースは自動です。
以下はローカルでの基本的な操作です。

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

PR のマージ後は Auto Release がタグを作成するため、通常は手動のタグ push は不要です。
手動で作成した `vMAJOR.MINOR.PATCH` タグの push からも同じ Release ワークフローを実行できます。

`.github/workflows/release.yml` がタグと Cargo のバージョンの一致、テスト、lint を確認し、
以下のアセットを生成します（各アーカイブに `.sha256` と `.version` が付属）。

- `tora-macos-arm64.tar.gz`
- `tora-macos-amd64.tar.gz`
- `tora-linux-amd64.tar.gz`

全ビルド完了後、すべてのアセットを draft release にアップロードしてから公開します。
GitHub の標準リリースノートを自動生成します。`vMAJOR.MINOR.PATCH` の安定版のみ対応します。
公開済みタグ・アセットは変更せず、修正は新しいバージョンとして出してください。
失敗した場合は該当 Actions run の失敗ジョブを再実行してください。
同じコミットの既存タグは再利用し、別コミットのタグは上書きせず停止します。
残っている draft はアセットを再アップロードして公開を再開できます。
公開済みの Release はアセットを変更せず成功として終了します。

GitHub Actions と `contents: write` 権限が必要です。公開ワンライナーの利用には
リポジトリと Release が公開されている必要があります。
