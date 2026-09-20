# 共通設定とメンテナンスガイド

このガイドは `serial` mode と `scheduled` mode の両方に適用されます。timezone、設定 snapshot、log、実行 policy、Web UI、データベース、更新、アンインストールについて説明します。

Job や Flow の使い方は、[serial mode ガイド](serial.ja.md) と [scheduled mode ガイド](scheduled.ja.md) を参照してください。

## Web UI

```bash
stoker ui start --open
stoker ui status
stoker ui stop
```

デフォルトでは Web UI は `127.0.0.1:8765` だけで待ち受けます。ローカルネットワークからアクセスする場合は、loopback ではないアドレスを明示的に設定し、信頼できるネットワークだけで使用してください。

```bash
stoker ui start --host 0.0.0.0 --port 8765
```

## timezone と設定 snapshot

時刻は SQLite に常に UTC で保存されます。`stoker jobs` と `stoker show` が時刻を表示するときにだけ変換されます。

初期化時に、Stoker は検出した IANA timezone を `~/.stoker/config.json` に保存します。

```bash
stoker config show
stoker config set timezone Asia/Taipei
stoker config get timezone
stoker config unset timezone
stoker config set timezone       # 対話式セレクターを開きます。
stoker config snapshot
stoker config restore
```

設定値は次の順番で解決されます。

1. CLI option
2. `config.json`
3. OS の timezone

`--timezone` または `--tz` を使うと、1 回の表示だけ timezone を上書きできます。

```bash
stoker jobs --tz Asia/Tokyo
stoker show <JOB_ID> --timezone UTC
```

設定を作成または更新すると、snapshot が `~/.stoker/snapshot/` に保存されます。設定が変わっていなくても、`config snapshot` は新しい snapshot を作成します。

## Log と実行 policy

デフォルトでは、1 つの Job の log は 64 MB までです。終了した Job 全体の log は 1024 MB までで、終了済み Job のうち最新 100 件の log が保持されます。

利用可能なディスク容量が 512 MB 未満になると、scheduler は次の Job を開始しません。

policy を変更する前に queue を lock してください。`STARTING`、`RUNNING`、`CANCELLING` の Job がないことも確認します。

```bash
stoker queue lock
stoker policy set log-max-bytes-per-job 256
stoker policy set log-retention-jobs 30
stoker policy set termination-grace-ms 30000
stoker policy set max-runtime-ms 43200000
stoker policy unset max-runtime-ms
stoker policy show
stoker queue unlock
```

log 容量は単位を付けない整数の MB で入力します。例は `256` です。

`stoker policy get <KEY>` を使うと、1 つの有効な値を確認できます。log が上限に達した場合や書き込みに失敗した場合も、Stoker は子プロセスの出力を読み取ります。ただし、古い log の一部が破棄されることがあります。

## データベース、更新、アンインストール

```bash
stoker db check
stoker db check --integrity
stoker db backup
stoker db backup <BACKUP_PATH>

# restore の前に scheduler を停止してください。
stoker db restore <BACKUP_PATH> --yes

stoker --version
stoker update
stoker uninstall
```

パスを指定しない場合、`db backup` は timestamp 付きの backup を `<STOKER_HOME>/backups/` に保存します。通常は `~/.stoker/backups/` です。backup には SQLite WAL も含まれます。

restore は現在のデータベースを置き換えます。scheduler が中断された場合、実行中の Job は `LOST` になり、queue が lock されます。Job を手動で確認してから `stoker queue unlock` を実行してください。

更新またはアンインストールの前に scheduler を停止してください。どちらの command も `--yes` を付けると確認を省略できます。

アンインストールしても Job のデータと log は削除されません。Cargo で特定バージョンをインストールする場合は、次を実行します。

```bash
cargo install stoker-engine --version <VERSION> --force
```
