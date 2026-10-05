# tora

便利なコマンドを追加していくための Rust 製 CLI。
[usagi](https://github.com/KKyosuke/usagi) の配布・更新方式を参考にした基盤です。

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

macOS（Apple Silicon / Intel）、Linux x86_64（glibc 2.35 以降）に対応します。
Windows、Linux ARM64、musl / Alpine は現時点では対象外です。

配置先やインストールする版を指定する場合：

```sh
curl -fsSL https://raw.githubusercontent.com/KKyosuke/tora/main/scripts/install.sh | TORA_HOME="$HOME/.local/share/tora" bash -s -- --version v0.1.0
```

`TORA_HOME` を変更した場合は、実行時にも同じ値を設定し、`$TORA_HOME/bin` を PATH に追加してください。
`TORA_VERSION=v0.1.0 bash scripts/install.sh` による指定もできます。

## 使い方

```sh
tora --help
tora --version
tora update                       # 最新の安定版を導入
tora update --version v0.1.0      # 指定版を導入（再インストール・ダウングレードも可）
tora completion zsh               # 補完スクリプトを標準出力へ
```

`update` は `TORA_HOME/bin/tora`（既定は `~/.tora/bin/tora`）から実行してください。
ソースビルドや別の配置先のバイナリは自己更新を拒否します。
更新時は実行中のバイナリに埋め込んだインストーラーを使います。
環境変数 `TORA_VERSION` は `tora update` では無視し、指定には `--version` を使います。

インストーラーは最新版のタグを一度だけ解決し、同じタグからアーカイブ、SHA-256、
バージョン情報を取得します。内容・チェックサム・実バイナリのバージョンを検証し、
同一ファイルシステム内の rename で置き換えます。検証失敗時は既存バイナリを保持します。
チェックサムは配布中の破損検知のためのもので、独立した署名ではありません。

同時更新は `$TORA_HOME/update.lock` で拒否します。通常の終了・シグナルでは自動解放します。
強制終了などでロックが残った場合は、他のインストーラーが動いていないことを確認してから
空の `update.lock` ディレクトリを `rmdir` で削除してください。

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
