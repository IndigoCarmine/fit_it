# fit_it

[English README](README.md)

[lmfit](https://lmfit.github.io/lmfit-py/) と [SasView](https://www.sasview.org/) を参考にした、デスクトップ用のフィッティングアプリです。データを読み込み、モデルを組み合わせ、曲線を見ながらパラメータを調整して、1 つまたは複数のデータセットをフィットできます。

- **組み合わせられるプリセット** — ピーク (Gaussian, Lorentzian, Voigt, pseudo-Voigt, Pearson VII, skewed Gaussian)、バックグラウンド、減衰、ステップ、振動、SAS モデル (Sphere, Guinier, Porod, Debye, Ornstein–Zernike)、超分子重合モデル (isodesmic、cooperative、核サイズ N の cooperative、isodesmic/cooperative 競合 — 濃度依存・温度依存)。成分は `(g1 + g2) * e1 + bg` のような式で結合でき、空欄なら総和です。
- **自作モデルを Python・C・数式で** — プリセット自体が普通のプラグインファイルなので、プリセットにできることはプラグインでもできます。
- **lmfit 流のパラメータ** — 値・範囲・vary・拘束式 (`2 * g1_sigma`)。
- **グローバルフィット** — 複数データセットを同時にフィットし、パラメータを共有 (`D1.g1_sigma`)。
- **統計量** — χ²、reduced χ²、AIC/BIC、R²、標準誤差 (拘束パラメータにも伝播)、相関を lmfit 形式のレポートで表示。ΔG(300 K) などの派生量も誤差伝播付きで出力。
- **UV-Vis ワークフロー** — JASCO のエクスポート (単一スペクトル・温度スキャン) を直接読み込み、断面ツールで「吸光度 vs 温度」「吸光度 vs 濃度」の曲線を作成。

## 使い方

1. **データを読み込む**: ウィンドウにドロップするか *File ▸ Open data*。CSV・TSV・空白区切りテキストに対応し、コメント行 (`#`, `%`, `;`) やヘッダーも扱えます。x / y / σ 列と重み付けを選びます。多列ファイルは「列ごとにデータセット」に分割できます。
2. **モデルを作る**: *➕ Add component* に読み込まれた全モデルがカテゴリ別に並びます。追加した成分の初期値は「データ − 既存成分」から推定されます (*Guess* で再推定)。成分名を変えたり、式で掛け算などに組み替えたりできます。
3. **調整する**: 値をドラッグするとプロットが即座に更新されます。範囲 (空欄 = 無制限)、*vary* のチェックで固定、拘束式の入力ができます。
4. **フィットする**: *▶ Fit* (Ctrl+Enter) は選択中のデータセットを、*▶▶ Global fit* はチェックした全データセットを同時にフィットします。*Undo fit* で元に戻せます。*Fit range* で範囲を制限できます (`= view` は表示中の x 範囲)。
5. **保存・書き出し**: *File ▸ Save project* (`*.fitit.json`、データ込み)。*File* からは、選択中データセットの CSV (データ・モデル・成分)、**グローバルフィット対象の全データセットを並べた CSV** (データセットごとに x, y, σ, モデル, 残差, 範囲内フラグ, 成分)、テキストのレポート、そして **PDF レポート** (全データセットの概観プロット、データセットごとのフィット曲線・成分・残差・パラメータ表のページ、派生量を含むフィットレポート全文。レポート横の *PDF…* からも可) を書き出せます。

UI は英語と日本語に対応しています (*言語* メニュー)。対数軸・成分表示・残差 (重み付き `(y − model)/σ`)・誤差棒・データセット重ね描きは *View* メニューにあります。

**定数 (Constants)**: 各データセットは名前付きの数値を持てます。読み込み時にファイル名を解析し、`..._50microM_...` から `conc_uM = 50`・`conc_M = 5e-5`、`THF200_Water300` から `THF = 200`・`Water = 300` を作ります。定数はパラメータ名と同様に、拘束式 (`c_tot` = `conc_M`)・他データセットの拘束 (`D2.conc_M`)・派生量で使えます。

**派生量 (Derived quantities)** (Fit パネル) はパラメータと定数の式 (例 `t1_deltaH - 300 * t1_deltaS`) で、値はリアルタイムに更新され、レポートの `[[Derived]]` に共分散からの誤差伝播付きで出力されます。

### UV-Vis: 超分子重合

*Supramolecular* プリセットは [sp_fitting_models](https://github.com/IndigoCarmine/sp_fitting_models) の移植です (同じ式・同じ二分法ソルバー、結果一致を確認済み)。戻り値は会合分率 × `scaler`:

| モデル | x | パラメータ |
| --- | --- | --- |
| `Isodesmic`, `Cooperative`, `CooperativeN`, `CoopIso` | 全濃度 (M) | K, σ, (N), scaler |
| `TempIsodesmic`, `TempCooperative`, `TempCooperativeN`, `TempCoopIso` | 温度 (K) | ΔH, ΔS, ΔH_nuc, (N), c_tot, scaler |

K = exp(−ΔH/RT + ΔS/R)、σ = exp(−ΔH_nuc/RT) です (正の ΔH_nuc が核形成ペナルティ。旧 `model_fitting` は符号が逆)。`c_tot` は固定で、データセットの `conc_M` が自動で入ります。

温度列が °C の曲線 (例 `temperature[c]`) を直接フィットするときは、結合式の下の *x → モデル* を *°C → K* にします。モデルには K が渡り、グラフとフィット範囲は °C のままです。*全体にコピー* で設定もコピーされます。

**温度スキャン (冷却・加熱曲線)**:
1. JASCO の `.txt` を開く (2 次元エクスポートは「波長 × 温度」の 1 データセット)。
2. *Data > Cross-section* (またはデータセット下の *Cross-section…*) でスキャンを選び、波長、ベースライン範囲 (例 400–∞)、*°C to K*、*normalise*、会合で吸光度が下がるなら *invert* を指定。*Create* でスキャンごとに曲線ができ、元のスキャンはグローバルフィットから外れます。
3. *Supramolecular > TempCooperative* を追加し *Copy to all*。`t1_deltaH`・`t1_deltaS`・`t1_deltaHnuc` の ⚙ メニューで *Share with all datasets*。必要ならフィット範囲を設定し *to all datasets*。
4. *Global fit*。*Derived quantities > Presets* から 300 K の ΔG / K_elong / K_c を追加。

**滴定 (濃度ごとに 1 スペクトル)**: 全ファイルを開き、*Cross-section* を *Titration* モードで x = `conc_uM` にし、必要なら *Divide by* `conc_uM` (濃度あたり吸光度)、*µM to M*、*normalise* を指定して *Cooperative* (または *Isodesmic*) でフィット。

### グローバルフィット

各データセットにはタグ (`D1`, `D2`, …) があります。パラメータのメニュー (⚙) の *Share with all datasets* で、他のデータセットの同名パラメータに `D1.g1_sigma` という拘束が入ります。データセットをまたぐ任意の式 (`0.5 * D1.g1_center + 3`) も書けます。*Copy to all* で全データセットに同じモデルを与え、初期値は各データから推定し直します。その後 *Global fit* で、チェックしたデータセットの自由パラメータを同時に最適化します。

## モデル: プリセットとプラグイン

形式は 1 つだけで、置き場所のフォルダが違うだけです。

| フォルダ | 場所 |
| --- | --- |
| Presets | アプリ横の `presets/` (インストール時は読み取り専用) |
| Plugins | Windows `%APPDATA%\fit_it\plugins`、macOS `~/Library/Application Support/fit_it/plugins`、Linux `~/.config/fit_it/plugins` (*Models ▸ Plugins & presets* で変更可) |

プリセットと同名のモデルをプラグインに置くと置き換わります — プリセットをプラグインフォルダにコピーして編集すればカスタマイズできます。*Models ▸ New … from template* でコメント付きの雛形を作成し、*Reload models* で変更を読み込みます。`.py`・`.c`・`.fexpr` をウィンドウにドロップするとプラグインフォルダにコピーされます。

| ファイル | 読み込み方 |
| --- | --- |
| `*.c` | 見つかった C コンパイラ (`CC`、MSVC、clang、gcc) で共有ライブラリにしてロード (`.build/` にキャッシュ) |
| `*.dll` `*.so` `*.dylib` | そのままロード (C ABI を実装していれば言語は問いません) |
| `*.py` | 手元の Python (numpy 必須) で実行 |
| `*.fexpr` | 数式。コンパイラ不要 |

### 数式モデル (`.fexpr`)

```toml
name = "StretchedExponential"
category = "Decay"
formula = "amplitude * exp(-(x / tau) ^ beta) + c"

[[params]]
name = "beta"
default = 0.7
min = 0.0
max = 1.0
```

`x` 以外の名前はすべてパラメータです。*Models ▸ New formula model* から対話的に作れます。

### Python モデル (SasView 形式)

```python
import numpy as np
name = "OrnsteinZernike"
category = "SAS"
description = "I(q) = scale / (1 + (q xi)^2) + background"
#            [name, units, default, [lower, upper], type, description]
parameters = [["cor_length", "Ang", 50.0, [0, np.inf], "", "screening length"]]

def Iq(q, cor_length):
    return 1.0 / (1.0 + (q * cor_length) ** 2)
```

`Iq` を使うと SasView と同様に `scale` と `background` が自動で追加されます (`form_volume` にも対応)。パラメータを既定で固定したり既定の拘束を付けるには `param_defaults = {"c_tot": {"vary": False, "expr": "conc_M"}}` を追加します。単純な関数なら `f(x, ...)` を定義してください。パラメータ行は `("a", 1.0)` や `("a", 1.0, lo, hi)` の短縮形も使え、任意の `guess(x, y)` で初期値を返せます。インタプリタは *Models ▸ Plugins & presets* (または環境変数 `FIT_IT_PYTHON`) で指定します。その `sys.path` を使うので、仮想環境のパッケージも import できます。

アプリ本体は Python をリンクしていません。`.py` モデルは初回使用時にロードされる小さなブリッジライブラリ (`fit_it_py`) を通じて実行されるので、Python が無い環境でも fit_it は起動します。ブリッジは Python の安定 ABI でビルドされるため、Windows では CPython 3.9 以上なら動作します。Linux ではビルド時と同じバージョンの Python が必要です。

### C モデル

```c
#include "fit_it_plugin.h"   /* プラグインフォルダに自動で置かれます */
#include <math.h>

static const FitItParam params[] = {
    {"amplitude", "", "", 1.0, -HUGE_VAL, HUGE_VAL},
    {"decay",     "", "", 1.0, 0.0,       HUGE_VAL},
};
static int32_t eval(const double *x, size_t n, const double *p, double *out) {
    for (size_t i = 0; i < n; ++i) out[i] = p[0] * exp(-x[i] / p[1]);
    return 0;
}
static const FitItModel models[] = {
    {FIT_IT_ABI_VERSION, 2, "MyDecay", "Custom", "amplitude exp(-x/decay)", params, eval, NULL},
};
FIT_IT_EXPORT const FitItModel *fit_it_models(uint32_t *count) { *count = 1; return models; }
```

1 ファイルで複数のモデルを公開でき、任意の `guess` 関数で初期値を与えられます。パラメータ行の末尾に `FIT_IT_PARAM_FIXED, "conc_M"` を付けると既定で固定・データセット定数に追従します (ABI v2。v1 のライブラリも読み込めます)。`.fexpr` では `[[params]]` に `vary = false`・`expr = "conc_M"` と書きます。完全な例は [`presets/*.c`](presets) を参照してください。

## フィットの詳細

数値ヤコビアンと Marquardt スケーリングを使う Levenberg–Marquardt 法です。範囲は lmfit/MINUIT と同じ変数変換で扱い、拘束式は依存順に評価します (循環はエラー表示)。標準誤差は lmfit の既定と同じく共分散 `(JᵀJ)⁻¹ · χ²ᵣ` から求めます。

## ビルド

```bash
cargo run --release            # アプリ + Python ブリッジ (ワークスペース既定)
cargo test --workspace
cargo build -p fit_it          # ブリッジをビルドする Python が無い場合はアプリのみ
```

`build.rs` はアプリがユーザープラグインに使うのと同じコードで `presets/*.c` を `target/<profile>/presets/.build` にコンパイルします。そのためビルドには C コンパイラが必要です (Windows は MSVC Build Tools、macOS は Xcode CLT、Linux は gcc/clang)。ブリッジのビルドには PATH 上の Python 3.9 以上が必要です。Linux ではさらに以下が必要です:

```bash
sudo apt-get install -y libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libwayland-dev libegl1-mesa-dev libgtk-3-dev libssl-dev python3-dev
```

サンプルデータと既成のプロジェクトは [`examples/`](examples) にあります。

## リリース

`v*` タグを push すると、Windows インストーラとポータブル zip、macOS ユニバーサル `.dmg` (Python モデルは Apple Silicon のみ)、Linux の AppImage・`.deb`・ポータブル tarball がビルドされます。詳細は [`.github/workflows/release.yml`](.github/workflows/release.yml)。インストーラは署名されていません。

## ライセンス

MIT — [LICENSE](LICENSE) を参照。

fit_it には [Noto Sans JP](https://github.com/notofonts/noto-cjk) (Regular) を同梱しています。画面の日本語表示と、PDF レポートへのサブセット埋め込みに使います。ライセンスは SIL Open Font License 1.1 です — [resources/fonts/OFL.txt](resources/fonts/OFL.txt) を参照。
