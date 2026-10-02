# wslc-simple-gcc

WSL Containers (`wslc.exe`) を使い、Windows の PowerShell から C/C++ を GCC 16 でコンパイルして実行するスクリプトです。

Docker Desktop や `dockerd` は不要です。WSL が管理するサービスと軽量 VM は使用します。生成するのは Linux 用バイナリです。

## 必要な環境

- Windows 上で動作する WSL Containers（WSL 2.9.3 以上）。
- **PowerShell 7.3 以上**。Windows PowerShell 5.1 には対応していません。
- 初回のイメージ取得時にコンテナレジストリへのネットワーク接続。

WSL は必要に応じて `wsl --update` で更新してください。WSL Containers の導入については [Microsoft の公式ドキュメント](https://learn.microsoft.com/en-us/windows/wsl/wsl-container) を参照してください。

スクリプトは PATH 上の `wslc.exe` を探し、見つからなければ `%ProgramFiles%\WSL\wslc.exe` を使用します。別の場所にある場合は `-WslcPath` を指定できます。

## すぐに試す

PowerShell 7 で実行します。

```powershell
git clone https://github.com/uni-kakurenbo/wslc-simple-gcc.git
Set-Location wslc-simple-gcc

# C23
.\Run-Gcc16.ps1 .\examples\hello.c

# C++23
.\Run-Gcc16.ps1 .\examples\hello.cpp

# C++26、最適化、プログラムへの引数
.\Run-Gcc16.ps1 .\examples\hello.cpp -Standard c++26 `
    -CompilerArgs @('-O2') -RunArgs @('hello world', '42')

# 直前の実行結果
$LASTEXITCODE
```

実行結果の例：

```text
[GCC 16.2.0 | c++23]
Hello C++! GCC 16.2.0; __cplusplus=202302; sum=15
```

コンパイラのバージョン表示は標準エラー、プログラムの通常出力は標準出力へ送られます。C++26 の機能サポートは GCC 16 の実装範囲に依存します。

## 自分のコードを実行する

```powershell
# 単一ソース（パスは呼び出し元の作業ディレクトリが基準）
.\Run-Gcc16.ps1 'C:\projects\demo\main.cpp'

# 複数ソースとヘッダーを含むプロジェクト
.\Run-Gcc16.ps1 -Source @('.\src\main.cpp', '.\src\util.cpp') `
    -ProjectDirectory . -CompilerArgs @('-Iinclude', '-O2', '-pthread')

# 対話入力が必要なプログラム
.\Run-Gcc16.ps1 .\main.c -Interactive

# 端末が必要なプログラム（Interactive も有効になる）
.\Run-Gcc16.ps1 .\main.c -Tty

# 引用符・空白・空文字を含む引数
.\Run-Gcc16.ps1 .\examples\hello.cpp `
    -RunArgs @('hello world', '', 'quote"test', 'semi;colon')
```

`-Source` と `-ProjectDirectory` は Windows のパスを受け取ります。`-CompilerArgs` 内のパスはコンテナ内のパスです。たとえば `-Iinclude` は `/src/include` を指します。

## パラメーター

| パラメーター | 内容 | 既定値 |
| --- | --- | --- |
| `-Source` | ソースファイルの配列。必須、第 1 位置引数 | — |
| `-ProjectDirectory` | `/src` にマウントするプロジェクトルート | 最初のソースの親フォルダー |
| `-Language` | `auto`、`c`、`cpp`。明示すると全ソースをその言語として扱う | `auto` |
| `-Standard` | `c23`、`c++23`、`c++26` など GCC の言語規格名 | C は `c23`、C++ は `c++23` |
| `-CompilerArgs` | コンパイラ・リンカーへの追加引数の配列 | 空 |
| `-RunArgs` | プログラムへの引数の配列 | 空 |
| `-Image` | GCC 16 と Bash を含む OCI イメージ | `docker.io/library/gcc:16.2.0` |
| `-WslcPath` | `wslc.exe` の明示的なパス | 自動検出 |
| `-Interactive` | コンソールの標準入力を接続 | 無効 |
| `-Tty` | TTY を割り当て、標準入力を接続 | 無効 |

詳しいヘルプは `Get-Help .\Run-Gcc16.ps1 -Full` でも確認できます。

## 動作と制約

- イメージがなければ取得し、以後はローカルキャッシュを使います。更新する場合は `wslc pull docker.io/library/gcc:16.2.0` を実行してください。
- 実際のコンパイラのメジャーバージョンが 16 であることを実行時に確認します。
- プロジェクトを `/src` に**読み取り専用**でマウントします。コンパイル・実行時の作業ディレクトリも `/src` です。
- ビルド結果はコンテナ内の `/tmp` に作り、終了時に `--rm` でコンテナごと削除します。ホストへのバイナリ保存は行いません。プログラムが書き込む一時ファイルにも `/tmp` を使用してください。
- `-ProjectDirectory` はローカル Windows ドライブ上のディレクトリで、すべてのソースを含む必要があります。UNC パスや `\\wsl$` のパスには対応していません。
- `.c` は C、`.C` / `.cc` / `.cpp` / `.cxx` / `.c++` は C++ として判定します。自動判定での C と C++ の混在はエラーになります。
- 同じ言語の複数ソースには対応していますが、CMake/Make、混在言語ビルド、外部ライブラリのインストールは対象外です。
- 引数をシェルのコードに埋め込まず、個別の引数として渡します。空白、引用符、空文字を含むプログラム引数を扱えます。

コンパイルに失敗するとプログラムを実行せず、その終了コードを返します。成功時はプログラムの終了コードを返します。スクリプトの入力チェックエラーは `2`、WSL の実行エラーは CLI の終了コードを返します。

## 検証

WSL Containers が利用できる Windows の PowerShell 7 で実行してください。

```powershell
.\tests\Smoke.Tests.ps1
```

C23・C++23・C++26、複数ソース、インクルードパス、引用符を含む引数、空文字、日本語・空白を含むパス、読み取り専用マウント、終了コードの伝播を実際のコンテナで確認します。テスト用ファイルは無視対象の `.tmp/` に作り、終了時に削除します。テストはイメージ取得・コンテナ起動を行うため、ネットワーク接続とローカルディスクを使用する場合があります。

動作確認環境：WSL 3.0.1、PowerShell 7.6.5、GCC 16.2.0。

## ライセンス

[MIT License](LICENSE)。
