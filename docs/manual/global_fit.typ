// fit_it 操作マニュアル: 熱力学パラメータのグローバルフィット
//
// スクリーンショットと例題データは tests/manual_screenshots.rs が生成します:
//   cargo test --test manual_screenshots -- --ignored --nocapture
//   typst compile docs/manual/global_fit.typ

#set document(title: "fit_it 操作マニュアル — 熱力学パラメータのグローバルフィット", author: "fit_it")
#set page(paper: "a4", margin: (x: 18mm, y: 20mm), numbering: "1 / 1")
#set text(font: ("BIZ UDPGothic", "Noto Sans JP"), size: 10pt, lang: "ja")
#set par(justify: true, leading: 0.8em)
#set heading(numbering: "1.1")
#show heading: set block(above: 1.4em, below: 0.8em)
#show heading.where(level: 1): it => {
  pagebreak(weak: true)
  it
}
#show raw: set text(font: ("Consolas", "BIZ UDGothic"), size: 8.5pt)
#show figure.caption: set text(size: 9pt)

// ---- helpers ----------------------------------------------------------------

/// スクリーンショット全体 (1360 × 820 px)。
#let shot(name, caption) = figure(
  box(stroke: 0.5pt + luma(180), image("images/" + name + ".png", width: 100%)),
  caption: caption,
)

/// 画面の一部の拡大図 (tests/manual_screenshots.rs の `Shots::crop` が切り出す)。
/// PNG に解像度情報がないので 1 px = 1 pt で置かれ、ページより広いものは縮小される。
#let crop(name, caption) = figure(
  box(stroke: 0.5pt + luma(180), image("images/" + name + ".png")),
  caption: caption,
)

#let ui(body) = box(
  fill: luma(235),
  inset: (x: 3pt, y: 1pt),
  outset: (y: 2pt),
  radius: 2pt,
  text(weight: "bold", body),
)

#let note(body) = block(
  fill: rgb("#eef5fc"),
  stroke: (left: 2pt + rgb("#2f7bd0")),
  inset: 8pt,
  width: 100%,
  body,
)

#let step(n, body) = grid(
  columns: (auto, 1fr),
  gutter: 6pt,
  box(fill: rgb("#2f7bd0"), inset: (x: 5pt, y: 2pt), radius: 8pt, text(fill: white, weight: "bold", str(n))),
  body,
)

// ---- title ------------------------------------------------------------------

#align(center)[
  #v(8mm)
  #text(size: 20pt, weight: "bold")[fit_it 操作マニュアル]
  #v(2mm)
  #text(size: 14pt)[熱力学パラメータのグローバルフィット]
  #v(1mm)
  #text(size: 11pt, fill: luma(90))[協同的（核形成–伸長）超分子重合モデル TempCooperative]
  #v(6mm)
]

このマニュアルでは、濃度の異なる複数の温度変化曲線（冷却曲線）を *1 組の熱力学パラメータ*
（伸長エンタルピー $Delta H$、伸長エントロピー $Delta S$、核形成ペナルティ $Delta H_"nuc"$）で同時に
説明する「グローバルフィット」を、fit_it の画面を追いながら順に説明します。

例題データはモデル式から作った *偽データ*（ノイズ入り）なので、フィット結果を「真の値」と
比べて手順が正しく働いていることを確かめられます。データは `docs/manual/data/` にあります。

#outline(title: "目次", indent: auto, depth: 2)

= 準備

== モデル: TempCooperative

TempCooperative は協同的（核形成–伸長）超分子重合の平衡モデルです。温度 $T$ における伸長の平衡定数
$K$ と協同性 $sigma$ を van 't Hoff 式で表します（$R = 8.314 "J/(mol K)"$）。

$ K(T) = exp(-(Delta H - T Delta S) / (R T)), quad sigma(T) = exp(-(Delta H_"nuc") / (R T)) $

全濃度 $c_"tot"$ と遊離モノマー濃度 $c_"m"$ の物質収支

$ c_"tot" = c_"m" + sigma / K dot ((K c_"m")^2 (2 - K c_"m")) / (1 - K c_"m")^2 $

を $c_"m"$ について解き、会合体の割合（凝集度）にスケール係数を掛けたものがモデルの出力です。

$ y(T) = "scaler" times (1 - c_"m" / c_"tot") $

#table(
  columns: (auto, auto, 1fr),
  inset: 5pt,
  stroke: 0.5pt + luma(180),
  table.header([*パラメータ*], [*単位*], [*意味*]),
  [`deltaH`], [J/mol], [伸長エンタルピー $Delta H$（負 = 発熱的に会合）],
  [`deltaS`], [J/(mol K)], [伸長エントロピー $Delta S$],
  [`deltaHnuc`], [J/mol], [核形成ペナルティ $Delta H_"nuc"$（正で協同的）],
  [`c_tot`], [M], [全濃度。*固定*。ファイル名から読んだ定数 `conc_M` に自動で連動],
  [`scaler`], [—], [完全に会合したときの信号強度（データセットごと）],
)

$Delta H, Delta S, Delta H_"nuc"$ は分子そのものの性質なので *全データセットで共通*、
`scaler` は測定ごとに違ってよいので *データセットごと* に求めます。これがグローバルフィットの考え方です。

== 例題データ（偽データ）

TempCooperative に下の「真の値」を入れて 283–373 K を 0.5 K 刻みで計算し、標準偏差 0.012 の
ガウスノイズを加えました。ファイル名の `_5microM_` などの部分から、fit_it が濃度を自動で読み取ります。

#table(
  columns: (1fr, auto, auto),
  inset: 5pt,
  stroke: 0.5pt + luma(180),
  table.header([*ファイル*], [*濃度*], [*scaler（真の値）*]),
  [`sample_5microM_cooling.txt`], [5 µM], [0.98],
  [`sample_10microM_cooling.txt`], [10 µM], [1.02],
  [`sample_20microM_cooling.txt`], [20 µM], [0.97],
  [`sample_50microM_cooling.txt`], [50 µM], [1.01],
)

共通の真の値: $Delta H = -90.0 "kJ/mol"$, $Delta S = -180 "J/(mol K)"$, $Delta H_"nuc" = 10.0 "kJ/mol"$。

ファイルの中身は、タブ区切りの 2 列（温度 [K] と凝集度）です。`#` で始まる行は無視されます。

```
# Synthetic cooling curve (TempCooperative + noise)
T_K	aggregated
283.15	0.98347
283.65	0.96821
...
```

#note[
  *実測データの場合:* JASCO の温度スキャン（波長 × 温度の 2 次元データ）は、データセット欄の
  #ui[断面…] で「ある波長での値 vs 温度」の曲線に変換してから、以下と同じ手順でフィットできます。
]

= 手順

== 起動とデータの読み込み

#step(1)[fit_it を起動します。起動直後はプリセットのモデルが読み込まれ、下のステータスバーに
「◯ 個のモデルを読み込みました」と表示されます。]

#shot("01_start", [起動直後の画面。左: データセットとモデル、中央: プロット、右: パラメータとフィット。])

#step(2)[4 つのデータファイルを *まとめてウィンドウにドロップ* します（または #ui[ファイル] →
#ui[データを開く…]）。データセットには D1〜D4 のタグが付きます。]

#shot("02_loaded", [4 つのファイルを読み込んだところ。])

#crop("c02_datasets", [データセット欄。ファイル名から `conc_M`（mol/L）と `conc_uM` が定数として読み取られている。])

左端のチェックボックスは「グローバルフィットに含める」の意味です。今回はすべてチェックしたままにします。

== モデルの設定（1 つ目のデータセット）

#step(3)[D1 を選び、#ui[➕ 成分を追加] → #ui[Supramolecular] → #ui[TempCooperative] を選びます。
項目にマウスを乗せると、モデルの説明とパラメータ一覧が表示されます。]

#shot("03_add_component", [成分の追加メニュー。])

#step(4)[成分 `t1` が追加され、右側にパラメータ表が現れます。プロットには現在のパラメータでの
モデル曲線（実線）と残差（下段）が表示されます。]

#crop("c04_params", [TempCooperative のパラメータ。`t1_c_tot` は拘束式 `conc_M` で濃度に連動し、「可変」がオフ（固定）になっている。])

#step(5)[*初期値と境界* を入力します。値の欄をクリックすると数値を直接入力でき、
最小・最大の欄には数値を入力して Enter で確定します。]

#table(
  columns: (auto, auto, auto, auto, 1fr),
  inset: 5pt,
  stroke: 0.5pt + luma(180),
  table.header([*パラメータ*], [*初期値*], [*最小*], [*最大*], [*理由*]),
  [`t1_deltaH`], [−80000], [−300000], [0], [会合は発熱的（$Delta H < 0$）],
  [`t1_deltaS`], [−150], [−1000], [0], [会合で自由度が減る（$Delta S < 0$）],
  [`t1_deltaHnuc`], [5000], [0], [50000], [正 = 核形成が不利（協同的）],
  [`t1_scaler`], [1], [0], [2], [正規化済みデータなので 1 前後],
)

#shot("05_bounds", [初期値と境界を入れたところ。初期値でも曲線はおおよそデータに沿っている。])

最小・最大の欄は幅が狭いため、`-300000` は `-3000`、`50000` は `5000(` のように途中までしか見えませんが、
値は正しく入っています（欄をクリックすると全体が表示されます）。

#note[
  初期値は「曲線がデータのおおよその位置に来る」程度で十分です。境界は物理的にあり得ない
  領域（例: 正の $Delta H$）を除く目的で設定します。後で使う大域的アルゴリズム（マルチスタート LM など）は、
  この *境界の範囲内* を探索します。
]

== 全データセットへのコピーとパラメータの共有

#step(6)[#ui[全体にコピー] を押すと、D1 のモデルとパラメータ設定（初期値・境界）が、
グローバルフィット対象のすべてのデータセットにコピーされます。]

#crop("c06_copy", [#ui[全体にコピー] ボタン（モデル欄）。])

#shot("07_copied_d3", [D3 を選択したところ。同じモデルが設定され、`t1_c_tot` は D3 の濃度（20 µM）に自動で合っている。])

#step(7)[D1 に戻り、パラメータ表の右端の #ui[⚙] → #ui[t1_deltaH を全データセットで共有] を選びます。
同じ操作を `t1_deltaS` と `t1_deltaHnuc` にも行います。*`t1_scaler` は共有しません。*]

#crop("c08_share_menu", [⚙ メニューの「全データセットで共有」。])

共有すると、D1 の ⚙ が ↔ に変わり、D2〜D4 の同じパラメータに拘束式 `D1.t1_deltaH` などが
自動で入ります。これで 3 つの熱力学パラメータは D1 の値ひとつに束ねられます。

#crop("c09_shared_d1", [D1: 共有元（⚙ が ↔ に変わる）。])

#crop("c10_shared_d2", [D2: 拘束式 `D1.t1_…` で D1 に連動（値は斜体）。])

== 派生量（300 K での値）

#step(8)[フィット欄の #ui[派生量] を開き、#ui[プリセット] から `deltaG_300K`、`K_elong_300K`、
`sigma_300K` を追加します。フィット後、これらは *誤差伝播付き* でレポートに出力されます。]

#crop("c11_derived_presets", [派生量のプリセット（TempCooperative 成分 `t1` 用）。])

#table(
  columns: (auto, 1fr),
  inset: 5pt,
  stroke: 0.5pt + luma(180),
  table.header([*派生量*], [*式*]),
  [`deltaG_300K`], [$Delta G = Delta H - 300 Delta S$],
  [`K_elong_300K`], [$K = exp(-Delta G \/ (R dot 300))$],
  [`K_c_300K`], [$K c_"tot"$（今回は使わない）],
  [`sigma_300K`], [$sigma = exp(-Delta H_"nuc" \/ (R dot 300))$],
)

自分で式を書くときは #ui[➕ 追加] を押し、名前と式を入力します。名前だけのパラメータは
選択中のデータセット、`D2.t1_scaler` のように書くと他のデータセットを参照します。

== フィットの設定と実行

#step(9)[（任意）#ui[オプション] を開き、*アルゴリズム* を選びます。既定の Levenberg–Marquardt は
初期値の近くの最小値を探す局所的な方法です。初期値に自信がないときや、結果が初期値によって変わるときは
*マルチスタート LM* を選ぶと、境界内の多数の点から LM を実行して最もよい解を採用します。]

#crop("c13_algorithm", [アルゴリズムの選択。])

#crop("c14_options", [マルチスタート LM の設定。「LM 開始点の数」「探索幅」「乱数シード」などが追加で表示される。])

#table(
  columns: (auto, 1fr),
  inset: 5pt,
  stroke: 0.5pt + luma(180),
  table.header([*アルゴリズム*], [*向いている場面*]),
  [Levenberg–Marquardt（局所）], [初期値がよいとき。最も速い],
  [マルチスタート LM], [局所解が心配なときの第一候補。評価回数あたりの効率がよい],
  [差分進化 + LM], [初期値の見当がまったくつかないとき。境界だけあればよいが、評価回数は多い],
  [ベイスンホッピング], [局所解から局所解へ跳び移りながら探す],
)

#note[
  大域的アルゴリズムは *境界の範囲内* を探索します。境界が無限（inf）のパラメータは
  「現在値 ± 探索幅 × max(|値|, 1)」の範囲しか探さないので、手順 5 のように有限の境界を入れておきます。
]

#step(10)[#ui[▶▶ グローバルフィット (4)] を押します。チェックされた 4 つのデータセットが同時にフィットされ、
共有パラメータはまとめて最適化されます（#ui[▶ D1 をフィット] は選択中のデータセットだけのフィットです）。
終わるとステータスバーに「グローバルフィット: 収束」と表示されます。]

#shot("15_result_d1", [グローバルフィット後（D1 を選択）。パラメータ表に値と誤差、右下にフィットレポート。残差はランダムに散らばっている。])

#crop("c15_params", [フィット後のパラメータ表（D1）。「± 誤差」列に標準誤差と相対誤差が入る。])

#shot("16_result_d4", [同じフィットで D4 を選択したところ。$Delta H$ などは D1 と同じ値（斜体 = 拘束式から計算）、`t1_scaler` は D4 独自の値。])

うまくいかなかったときは #ui[フィットを元に戻す] で、フィット前のパラメータに戻せます。

== 結果の確認

#step(11)[#ui[表示] → #ui[グローバルフィット対象を重ねて表示] をオンにすると、4 本の曲線を重ねて比較できます。
濃度が高いほど高温側で会合が始まる様子を、1 組の熱力学パラメータで再現できていることがわかります。]

#shot("17_overlay", [全データセットの重ね表示。])

フィットレポート（抜粋）:

#{
  let lines = read("data/fit_report.txt").split("\n")
  let keep = lines.filter(l => not l.starts-with("    D2.") and not l.starts-with("    D3.") and not l.contains("C(D1.t1_scaler") and not l.contains("C(D2."))
  block(fill: luma(245), inset: 8pt, width: 100%, raw(keep.slice(0, calc.min(keep.len(), 40)).join("\n")))
}

真の値との比較:

#table(
  columns: (auto, auto, auto, auto),
  inset: 5pt,
  stroke: 0.5pt + luma(180),
  align: (left, right, right, right),
  table.header([*パラメータ*], [*真の値*], [*フィット結果*], [*相対誤差*]),
  [$Delta H$ (J/mol)], [−90000], [−89867 ± 294], [0.33 %],
  [$Delta S$ (J/(mol K))], [−180], [−179.56 ± 0.92], [0.51 %],
  [$Delta H_"nuc"$ (J/mol)], [10000], [10163 ± 101], [0.99 %],
  [scaler D1 / D2 / D3 / D4], [0.98 / 1.02 / 0.97 / 1.01], [0.979 / 1.022 / 0.969 / 1.009], [≤ 0.2 %],
  [$Delta G_(300 "K")$ (J/mol)], [−36000], [−35999 ± 24], [0.07 %],
)

どの値も誤差（±1σ）の 1〜2 倍以内で真の値に一致しています。

#note[
  *相関に注意:* レポートの `[[Correlations]]` で $Delta H$ と $Delta S$ の相関係数は +0.9988 と非常に大きく、
  両者は個別には決まりにくい（片方を動かすともう片方が追従する）ことを示しています。
  一方、両者の組み合わせである $Delta G_(300 "K")$ は 0.07 % という高い精度で決まっています。
  濃度の異なるデータを増やす、温度範囲を広げるなどで相関を下げられます。
]

== 保存と書き出し

#step(12)[#ui[ファイル] メニューから、プロジェクトの保存と結果の書き出しができます。]

#crop("c18_file_menu", [ファイルメニュー。])

- *プロジェクトを保存*（Ctrl+S）: データ・モデル・パラメータ（フィット後の値）・共有設定・派生量をまとめて保存します。
- *データとフィット曲線を書き出し (CSV)*: 選択中のデータセットのデータとモデル曲線。
- *グローバルフィットの全データを並べて書き出し (CSV)*: 全データセットを横に並べた CSV。グラフ作成ソフト向け。
- *フィットレポートを書き出し*: 上のレポートをテキストで保存。
- *PDF レポートを書き出し*: 各データセットのプロットとパラメータ表、レポートを 1 つの PDF に。

= よくある問題

#table(
  columns: (auto, 1fr),
  inset: 6pt,
  stroke: 0.5pt + luma(180),
  table.header([*症状*], [*対処*]),
  [`t1_c_tot` が 0 のまま], [ファイル名に `_50microM_` / `12.5uM` / `2mM` などの濃度表記がないと `conc_M` が作られません。データセット欄の *定数* で `conc_M` を手で追加します（単位は mol/L）。],
  [曲線がまったく合わない], [まず初期値を調整して、曲線がデータのおおよその位置に来るようにします。遷移温度が高すぎる／低すぎるときは $Delta S$ を、遷移の鋭さは $Delta H_"nuc"$ を動かすと効きます。],
  [結果が初期値によって変わる], [局所解に落ちています。手順 9 でマルチスタート LM（または差分進化 + LM）を選び、境界を有限にしてから再実行します。],
  [一部のデータだけ外したい], [データセット名の左のチェックを外すと、グローバルフィットから除外されます（パラメータは固定値として扱われます）。],
)

#v(1fr)
#text(size: 8.5pt, fill: luma(100))[
  このマニュアルのスクリーンショットと例題データは `cargo test --test manual_screenshots -- --ignored` で
  再生成できます（`tests/manual_screenshots.rs`）。PDF は `typst compile docs/manual/global_fit.typ` で作成します。
]
