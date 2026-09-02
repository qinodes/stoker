# stoker

[![Rust](https://img.shields.io/badge/built%20with-Rust-orange?logo=rust)](https://www.rust-lang.org/)
[![Platforms](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-blue)]
[![Version](https://img.shields.io/badge/version-0.2.0-informational)](Cargo.toml)

[English](README.md) | [繁體中文](README.zh-TW.md) | 日本語

**stoker はローカル command 向けのジョブスケジューラーです。**
各 Job は作成したときのディレクトリで実行され、`.stoker/runs` にはログだけが保存されます。環境と成果物はユーザー自身が管理します。

## クイックスタート

Rust と、command を実行するディレクトリが必要です。

```bash
# crates.io から CLI をインストール
cargo install stoker-engine

# scheduler をバックグラウンドで起動（Linux / Windows）
stoker serve

# 現在のディレクトリで DRAFT Job を作成
stoker submit --user alice --name exp-a --cmd python train.py --lr 0.0001

# 内容を確認して queue に追加（<JOB_ID> は直前の出力を使用）
stoker show <JOB_ID>
stoker commit <JOB_ID>

# 確認と管理
stoker status
stoker jobs
stoker jobs --user alice
stoker jobs --state draft
stoker jobs --state queued
stoker logs <JOB_ID>
stoker logs -f <JOB_ID>
stoker cancel <JOB_ID>
stoker stop
stoker --version
stoker update
stoker uninstall
```

`--cmd` 以降はすべて実行プログラムへそのまま渡されます。stoker 自身のオプションは `--cmd` より前に指定してください。

`--user` は論理的な owner ラベルであり、OS アカウントや認証機能ではありません。同じマシンを複数人で使う場合に、ジョブの識別と絞り込みに利用できます。

`jobs` は queue の概要として、`queue_order`、`job_id`、owner、名前、状態、時刻を表示します。QUEUED Job は `queue_order` 順、それ以外の状態は `-` です。`--state draft` または `--state queued` で絞り込めます。

Job の詳細、command、実行パスは `stoker show <JOB_ID>` で確認できます。

## Job の状態とキャンセル

| 状態 | 説明 |
| --- | --- |
| `DRAFT` | submit 済みですが、まだ queue に commit されていません。 |
| `QUEUED` | commit 済みで、実行待ちです。 |
| `STARTING` | scheduler が Job を取得し、ソースディレクトリとプロセスを準備しています。 |
| `RUNNING` | Job のプロセスが実行中です。 |
| `CANCELLING` | キャンセル済みで、stoker がプロセスの停止とクリーンアップを行っています。 |
| `SUCCEEDED` | Job が正常に完了しました。 |
| `FAILED` | Job のプロセスが失敗したか、stoker が実行を完了できませんでした。 |
| `CANCELLED` | Job はキャンセルされました。 |
| `LOST` | scheduler の再起動時に、実行中だった Job の管理状態が失われました。 |

`stoker cancel <JOB_ID>` を使うと、実行前の `DRAFT` または `QUEUED` Job をキャンセルでき、`STARTING` または `RUNNING` Job を停止することもできます。実行中の Job をキャンセルすると、まず `CANCELLING` になり、stoker が Job のプロセスツリーを終了してクリーンアップを待った後、`CANCELLED` として記録します。`cancel` を使うには scheduler が実行中である必要があります。

`stoker stop` は scheduler を停止し、現在実行中の Job をキャンセルします。`QUEUED` の Job は保持され、次回 scheduler を起動したときに処理されます。

Job は実行開始時点のソースディレクトリの内容を使用し、command による変更はそこに残ります。stoker はディレクトリ内のファイルを自動で変更または復元しません。

`stoker update` は更新可能なバージョンを表示し、明示的に `y` または `yes` を入力した場合のみ更新します。

特定バージョンを入れるには、`cargo install stoker-engine --version <VERSION> --force` を使用します。

`stoker uninstall` も明示的な確認が必要です。先に scheduler を停止してください。Job データと logs は Stoker のデータフォルダー（通常は `~/.stoker`）に保持されます。

## 対象範囲と制限

- 単一マシンの queue のみ対応。複数マシン、リモート実行、分散学習、GPU 割り当て、コンテナのスケジューリングには対応しません。
- submit 時のディレクトリが存在している必要があります。ディレクトリ内のファイル変更は検査しません。
- Python/Conda/CUDA 環境、データセット、checkpoint、artifact、実験メトリクスは管理しません。
- stoker のアカウント、ログイン、権限管理はありません。`--user` は識別と絞り込み専用です。
