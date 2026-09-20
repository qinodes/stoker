# serial mode 詳細ガイド

このガイドでは `serial` mode について説明します。各 Job は 1 回だけ実行され、queue の順番に従って 1 件ずつ処理されます。
[README](../README.ja.md) も参照してください。定期実行や task の依存関係が必要な場合は、[scheduled mode ガイド](scheduled.ja.md) を参照してください。

## Job の作成と commit

command を実行したいディレクトリで DRAFT Job を作成します。`--cmd` の後ろに指定する command 全体は、引用符で囲んでください。

```bash
stoker create --user <USER> --name <NAME> --cmd "<COMMAND>"
stoker show <JOB_ID>

# 指定した DRAFT Job を、記載した順番で commit します。
stoker commit <JOB_ID> [<JOB_ID>...]

# すべての DRAFT Job、または指定した user の DRAFT Job を作成時刻順で commit します。
stoker commit --all
stoker commit --user <USER>
```

`--user` は識別と絞り込みに使う論理的な owner ラベルです。OS アカウントや認証の仕組みではありません。

Job はバックグラウンドで実行され、対話型ターミナルを持ちません。対話操作を必要としない command と option を使ってください。

Linux と macOS では `sh`、Windows では `cmd.exe` で command を実行します。shell の構文や利用できるプログラムは OS によって異なる場合があります。

```bash
# Job の確認と絞り込み
stoker jobs
# stoker jobs [絞り込み条件]
stoker jobs --user alice
stoker jobs --state queued
stoker jobs --user alice --state failed

# 既存の log を表示、または Job が終了するまで追跡
stoker logs <JOB_ID>
stoker logs -f <JOB_ID>

# DRAFT、QUEUED、STARTING、RUNNING、CANCELLING の Job を cancel
stoker cancel <JOB_ID>
```

## Scheduler と queue

```bash
stoker start
stoker status
stoker stop
```

`stoker stop` の実行時に active Job があると、強制 cancel するか確認されます。`--yes` を付けると確認を省略できます。`QUEUED` Job は次に scheduler を起動するまで残ります。

Job の順番を変更する前に queue を lock してください。終わったら明示的に unlock します。queue が locked の間は commit できませんが、Job の作成と cancel はできます。

```bash
stoker queue lock
stoker queue edit
stoker queue unlock
```

`queue edit` に表示されるのは `QUEUED` Job だけです。閲覧モードでは `↑` と `↓` で選択し、`Enter` で移動モードに入り、`q` または `Esc` で queue を lock したまま終了します。

移動モードでは `↑` と `↓` で位置を変更します。`Enter` で移動を確定します。`q` または `Esc` で今回の移動を元に戻します。

| State | 意味 |
| --- | --- |
| `DRAFT` | 作成済みですが、queue に commit されていません。 |
| `QUEUED` | commit 済みで、実行を待っています。 |
| `STARTING` | scheduler が Job の準備をしています。 |
| `RUNNING` | Job のプロセスが実行中です。 |
| `CANCELLING` | cancel が要求され、プロセスとリソースを終了・整理しています。 |
| `SUCCEEDED` | Job が正常に完了しました。 |
| `FAILED` | Job のプロセスが失敗したか、stoker が実行を完了できませんでした。 |
| `CANCELLED` | Job が cancel されました。 |
| `LOST` | scheduler の再起動後、実行中だった Job の管理情報を失いました。 |

`stoker clean` を使うと、`SUCCEEDED`、`FAILED`、`CANCELLED`、`LOST` の Job とその log を削除できます。scheduler の実行中でも使用できます。

## Docker Job

次の Job を開始する前に container の終了を待たせる場合は、Docker を foreground モードで実行します。

```bash
docker run <IMAGE> <COMMAND>
```

`docker run -d` は使わないでください。container の起動直後に command が終了するため、Stoker は Job が完了したと判断します。
