# fit_it

[日本語版 README](README_ja.md)

A desktop curve-fitting workbench in the spirit of [lmfit](https://lmfit.github.io/lmfit-py/) and [SasView](https://www.sasview.org/): drop in data, combine models, tune parameters while watching the curve, and fit one dataset or many at once.

- **Preset models you can combine** — peaks (Gaussian, Lorentzian, Voigt, pseudo-Voigt, Pearson VII, skewed Gaussian), backgrounds, decays, steps, oscillations, SAS models (Sphere, Guinier, Porod, Debye, Ornstein–Zernike) and supramolecular polymerization models (isodesmic, cooperative, cooperative with nucleus size N, competing isodesmic/cooperative — vs concentration or vs temperature). Components combine with a formula such as `(g1 + g2) * e1 + bg`; empty means their sum.
- **Your own models, in Python, C or as a formula** — presets are ordinary plugin files, so anything a preset can do, a plugin can do too.
- **lmfit-style parameters** — value, bounds, vary, and constraint expressions (`2 * g1_sigma`).
- **Global fits** — several datasets fitted together, with parameters shared across them (`D1.g1_sigma`).
- **Honest statistics** — χ², reduced χ², AIC/BIC, R², standard errors (propagated to constrained parameters) and correlations, in an lmfit-style report, plus your own derived quantities (e.g. ΔG at 300 K) with propagated errors.
- **UV-Vis workflows** — JASCO exports (single spectra and temperature scans) load directly; a cross-section tool turns them into absorbance-vs-temperature or absorbance-vs-concentration curves.

## Using it

1. **Load data**: drop files on the window or *File ▸ Open data*. CSV, TSV and whitespace-separated text work; comment lines (`#`, `%`, `;`) and headers are handled. Pick the x / y / σ columns and the weighting. A multi-column file can be split into one dataset per column.
2. **Build the model**: *➕ Add component* lists every loaded model by category. New components get initial values guessed from the data minus the components already present (*Guess* redoes it). Rename components; edit the formula to multiply or combine them.
3. **Adjust**: drag values and the plot updates live. Set bounds (empty = unbounded), untick *vary* to fix a value, or type a constraint.
4. **Fit**: *▶ Fit* (Ctrl+Enter) fits the selected dataset; *▶▶ Global fit* fits all checked datasets together. *Undo fit* restores the previous values. Restrict the fit with *Fit range* (`= view` takes the visible x range).
5. **Save and export**: *File ▸ Save project* (`*.fitit.json`, data included). *File* also exports the selected dataset (data + model + components, CSV), **all global-fit datasets side by side** in one CSV (per dataset: x, y, σ, model, residual, in-range flag, components), the report as text, and a **PDF report**: an overview plot of all datasets, one page per dataset with the fitted curve, components, residuals and parameter table, and the full fit report with derived quantities (also *PDF…* next to the report).

The UI is available in English and Japanese (*Language* menu). Log axes, components, residuals (weighted, `(y − model)/σ`), error bars and dataset overlay are under *View*.

**Constants.** Each dataset can carry named numbers (its *Constants* section). File names are scanned on load: `..._50microM_...` gives `conc_uM = 50` and `conc_M = 5e-5`; tokens such as `THF200_Water300` give `THF = 200`, `Water = 300`. Constants work anywhere a parameter name does — in constraints (`c_tot` = `conc_M`), in other datasets' constraints (`D2.conc_M`) and in derived quantities.

**Derived quantities** (Fit panel) are expressions of fitted parameters and constants, e.g. `t1_deltaH - 300 * t1_deltaS`; they update live and appear in the report under `[[Derived]]` with errors propagated through the covariance.

### UV-Vis: supramolecular polymerization

The *Supramolecular* presets are a port of [sp_fitting_models](https://github.com/IndigoCarmine/sp_fitting_models) (same equations and bisection solver; verified against it). They return the aggregated fraction × `scaler`:

| Model | x | Parameters |
| --- | --- | --- |
| `Isodesmic`, `Cooperative`, `CooperativeN`, `CoopIso` | total concentration (M) | K, σ, (N), scaler |
| `TempIsodesmic`, `TempCooperative`, `TempCooperativeN`, `TempCoopIso` | temperature (K) | ΔH, ΔS, ΔH_nuc, (N), c_tot, scaler |

with K = exp(−ΔH/RT + ΔS/R) and σ = exp(−ΔH_nuc/RT) (a positive ΔH_nuc is a nucleation penalty; the older `model_fitting` package used the opposite sign). `c_tot` is fixed and set to the dataset's `conc_M` automatically.

If the temperature column is already a curve in °C (e.g. `temperature[c]`), set *x → model* under the formula to *°C → K*: the models then get kelvin while the plot and fit range stay in °C. *Copy to all* carries the setting along.

**Temperature scans (cooling/heating curves):**
1. Open the JASCO `.txt` files (a 2-D export is one dataset: wavelength × temperature).
2. *Data > Cross-section* (or *Cross-section…* under the dataset): tick the scans, set the wavelength, a baseline range (e.g. 400–∞), *°C to K*, *normalise*, and *invert* if absorbance falls on aggregation. *Create* makes one curve per scan and unchecks the raw scans from the global fit.
3. Add *Supramolecular > TempCooperative*, *Copy to all*, and in the ⚙ menu of `t1_deltaH`, `t1_deltaS`, `t1_deltaHnuc` choose *Share with all datasets*. Optionally set a fit range and *to all datasets*.
4. *Global fit*. Add ΔG / K_elong / K_c at 300 K from *Derived quantities > Presets*.

**Titrations (one spectrum per concentration):** open all files, *Cross-section* in *Titration* mode with x = `conc_uM`, optionally *Divide by* `conc_uM` (absorbance per concentration), *µM to M* and *normalise*, then fit *Cooperative* (or *Isodesmic*).

### Global fitting

Each dataset has a tag (`D1`, `D2`, …). A parameter menu (⚙) offers *Share with all datasets*, which sets `D1.g1_sigma` as the constraint of `g1_sigma` in the others; any expression across datasets works (`0.5 * D1.g1_center + 3`). *Copy to all* gives every dataset the same model and re-guesses starting values from each dataset's own data. *Global fit* then optimises all free parameters of the checked datasets simultaneously.

## Models: presets and plugins

There is one model format; only the folder differs.

| Folder | Where |
| --- | --- |
| Presets | `presets/` next to the app (read-only in an installation) |
| Plugins | Windows `%APPDATA%\fit_it\plugins`, macOS `~/Library/Application Support/fit_it/plugins`, Linux `~/.config/fit_it/plugins` (changeable in *Models ▸ Plugins & presets*) |

A plugin with the same model name as a preset replaces it — copy a preset into the plugin folder to customise it. *Models ▸ New … from template* creates a commented starting file; *Reload models* picks up changes. Dropping a `.py`, `.c` or `.fexpr` file on the window copies it into the plugin folder.

| File | How it is loaded |
| --- | --- |
| `*.c` | compiled to a shared library with the C compiler found (`CC`, MSVC, clang, gcc), cached in `.build/` |
| `*.dll` `*.so` `*.dylib` | loaded directly (any language exporting the C ABI) |
| `*.py` | run in your own Python (with numpy) |
| `*.fexpr` | formula, no compiler needed |

### Formula models (`.fexpr`)

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

Every name except `x` is a parameter. *Models ▸ New formula model* builds one interactively.

### Python models (SasView format)

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

With `Iq`, `scale` and `background` are added as in SasView (and `form_volume` is honoured). To make a parameter fixed by default or give it a default constraint, add `param_defaults = {"c_tot": {"vary": False, "expr": "conc_M"}}`. Define `f(x, ...)` instead for a plain function; short parameter rows `("a", 1.0)` or `("a", 1.0, lo, hi)` work too, and an optional `guess(x, y)` supplies starting values. Set the interpreter under *Models ▸ Plugins & presets* (or `FIT_IT_PYTHON`); its `sys.path` is used, so packages from a virtual environment are importable.

The app itself never links Python: `.py` models run through a small bridge library (`fit_it_py`) loaded on first use, so fit_it starts fine on machines without Python. The bridge is built against Python's stable ABI, so on Windows any CPython ≥ 3.9 works; on Linux the Python version must match the one it was built with.

### C models

```c
#include "fit_it_plugin.h"   /* written into the plugin folder for you */
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

One file may export several models; the optional `guess` function fills starting values. A parameter row may end with `FIT_IT_PARAM_FIXED, "conc_M"` to be fixed by default and follow a dataset constant (ABI v2; v1 libraries still load). In a `.fexpr`, the same is `vary = false` and `expr = "conc_M"` under `[[params]]`. See [`presets/*.c`](presets) for complete examples.

## Fitting details

Levenberg–Marquardt with numerical Jacobian and Marquardt scaling. Bounds use lmfit/MINUIT transforms; constraints are evaluated in dependency order (cycles are reported). Standard errors come from the covariance `(JᵀJ)⁻¹ · χ²ᵣ`, as in lmfit's default.

## Building

```bash
cargo run --release            # app + Python bridge (workspace default)
cargo test --workspace
cargo build -p fit_it          # app only, when no Python is available to build the bridge
```

`build.rs` compiles `presets/*.c` into `target/<profile>/presets/.build`, using the same code the app uses for user plugins — so a C compiler is needed to build (MSVC Build Tools on Windows, Xcode CLT on macOS, gcc/clang on Linux). Building the bridge needs a Python ≥ 3.9 on PATH. Linux also needs the usual windowing headers:

```bash
sudo apt-get install -y libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev \
  libxkbcommon-dev libwayland-dev libegl1-mesa-dev libgtk-3-dev libssl-dev python3-dev
```

Example data and a ready-made project are in [`examples/`](examples).

## Releases

Pushing a `v*` tag builds a Windows installer and portable zip, a macOS universal `.dmg` (Python models on Apple Silicon only), and a Linux AppImage, `.deb` and portable tarball; see [`.github/workflows/release.yml`](.github/workflows/release.yml). Installers are unsigned.

## License

MIT — see [LICENSE](LICENSE).

fit_it includes [Noto Sans JP](https://github.com/notofonts/noto-cjk) (Regular), used for Japanese text on screen and embedded as a subset in PDF reports. It is licensed under the SIL Open Font License 1.1 — see [resources/fonts/OFL.txt](resources/fonts/OFL.txt).
