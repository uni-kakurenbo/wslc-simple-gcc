# wslc-simple-gcc

Windows の WSL Containers を使い、C・C++・アセンブリを GCC 16 でコンパイルして実行する Rust CLI です。Rust から呼び出す API も同じ実装を利用します。

ソースの結合やテンプレート変換は行いません。複数ソースは GCC に個別に渡します。

## 必要な環境

- x64 Windows と WSL Containers（WSL 2.9.3 以上）。
- ビルド・インストールには公式の Rust 1.99.0 ツールチェーン。
- 初回の SDK・GCC イメージ取得にはネットワーク接続。

インストール後は Rust、PowerShell、Docker Desktop、ユーザー側の Linux ディストリビューションを必要とせず、実行ファイルを直接呼び出せます。WSL のサービスと軽量 VM は利用します。生成・実行するバイナリは Linux 用です。

WSL Containers の導入は [Microsoft の公式ドキュメント](https://learn.microsoft.com/en-us/windows/wsl/wsl-container) を参照してください。

## インストール

Rust 1.99 以上の環境で、公開リポジトリからインストールできます。

```sh
cargo install --git https://github.com/uni-kakurenbo/wslc-simple-gcc --locked wslc-simple-gcc
wslc-simple-gcc --help
wslc-simple-gcc doctor
```

クローンしたリポジトリからは次のようにインストールします。公式ツールチェーンは `rust-toolchain.toml` に固定しています。

```sh
cargo install --path crates/wslc-simple-gcc --locked
```

## コンパイルして実行する

```sh
wslc-simple-gcc examples/hello.c
wslc-simple-gcc examples/hello.cpp --standard c++26 --compiler-arg -O2 -- "hello world" 42
wslc-simple-gcc main.s
```

最初のソースの親ディレクトリを `/src` に読み取り専用でマウントします。ソースは呼び出し元の作業ディレクトリを基準に指定します。複数ソースを含む場合は必要に応じてプロジェクトルートを指定してください。

```sh
wslc-simple-gcc src/main.c src/util.c --project-dir . --compiler-arg -Iinclude --compiler-arg -O2
```

`--compiler-arg` と `--run-arg` は引数ごとに繰り返します。`--` 以降はすべてプログラムの引数です。空白、引用符、空文字を含む値も引数の区切りを保って渡します。

```sh
wslc-simple-gcc examples/hello.cpp --run-arg "hello world" --run-arg "" --run-arg "semi;colon"
wslc-simple-gcc examples/hello.c --interactive
```

## オプション

| オプション | 内容 | 既定値 |
| --- | --- | --- |
| `--project-dir` | `/src` にマウントするルート | 最初のソースの親 |
| `--language` | `auto` / `c` / `cpp` / `asm` | `auto` |
| `--standard` | GCC の C/C++ 規格名 | `c23` / `c++23` |
| `--compiler-arg` | コンパイラ・リンカー引数。繰り返し可能 | 空 |
| `--run-arg` | プログラム引数。繰り返し可能 | 空 |
| `--image` | GCC 16 と Bash を含む OCI イメージ | `docker.io/library/gcc:16.2.0` |
| `--cache-dir` | SDK とセッションの保存先 | `%LOCALAPPDATA%/wslc-simple-gcc` |
| `--wslc` | `wslc.exe` の明示的なパス | PATH、次に `%ProgramFiles%/WSL/wslc.exe` |
| `--timeout-seconds` | コンパイル・実行の制限時間 | `1800` |
| `--interactive` | コンソールの標準入力を接続 | 無効 |
| `--tty` | TTY を割り当て、標準入力を接続 | 無効 |

実際のコンパイラのメジャーバージョンが 16 であることを検査します。コンパイラの情報と SDK・セッションの診断は標準エラー、プログラムの通常出力は標準出力に送ります。コンパイルに失敗した場合はその終了コード、成功した場合はプログラムの終了コードを返します。入力エラーは `2`、実行基盤のエラーは `1` です。

## 実行とキャッシュ

Microsoft.WSL.Containers 3.0.1 の検証済み SDK を必要に応じて取得し、SHA-256 を確認してから DLL を読み込みます。このバージョンの C ABI に固定しています。

各呼び出しが専用セッションを所有し、正常終了・エラー・タイムアウト時に終了と解放を行います。既定セッションや他のアプリのセッションには変更を加えません。同じキャッシュに対する同時操作はロックで防ぎます。CLI 自体を強制終了した場合の SDK による後処理は保証できません。

GCC イメージと SDK はキャッシュに保存します。初回のイメージ取得はディスクと時間を使用します。コンパイル結果はコンテナ内の `/tmp` に作り、`--rm` でコンテナとともに削除します。ホストへの実行ファイルの書き出しは行いません。

プロジェクトはローカル Windows ドライブ上に置き、選択するソースをすべて含める必要があります。UNC パスや WSL のネットワークパスには対応していません。自動判定で C・C++・アセンブリを混在させることはできません。CMake/Make や外部ライブラリの導入は対象外です。

## Rust の責務

```text
crates/
  wslc-runtime/
    src/process.rs      プロセスの入出力・終了コード・タイムアウト
    src/sdk.rs          HTTPS 取得・ハッシュ検証・SDK キャッシュ
    src/session.rs      SDK 読み込み・専用セッションの所有権
    src/container.rs    マウント・引数・コンテナ実行
  wslc-simple-gcc/
    src/compile.rs      ソース選択・GCC 引数・コンパイルと実行の API
    src/native.rs       既存環境での GCC・アセンブリ実行補助
    src/cli.rs          CLI の引数解析
    src/main.rs         CLI の入口
    assets/compile-run.sh  コンテナ内で GCC を呼び出す固定手順
```

Rust の呼び出し元は `SessionOptions` でキャッシュなどを指定し、`with_session` の範囲で `compile::Request` または汎用の `RunOptions` を実行します。アプリの設定ファイル、データセット、ソース構成には依存しません。`process` と `native` の補助は Windows と Linux の呼び出し元で利用できます。

## 検証

```sh
cargo test --workspace --locked
cargo fmt --all -- --check
cargo clippy --workspace --locked --all-targets -- -D warnings
```

実際の WSLC を使う統合テストは通常のテストから分離しています。

```sh
cargo test --locked --package wslc-simple-gcc --test containers -- --ignored --test-threads 1
```

C23・C++23・C++26、複数ソース、インクルードパス、引用符・空文字・日本語の引数、読み取り専用マウント、終了コード、タイムアウト後のセッション解放、SDK キャッシュの修復を検証します。テストは新規の合成ソースとこのリポジトリのサンプルを使用します。`WSLC_GCC_TEST_CACHE` でテストのキャッシュを変更できます。

## ライセンス

[MIT License](LICENSE)。
