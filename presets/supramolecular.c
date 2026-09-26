/*
 * fit_it preset: supramolecular polymerization models.
 *
 * A port of sp_fitting_models (https://github.com/IndigoCarmine/sp_fitting_models,
 * src/lib.rs): the same mass balances, solved for the free monomer by bisection
 * (100 steps), with R = 8.314 J/(mol K). Every model returns the aggregated
 * fraction times `scaler`.
 *
 * Concentration models take x = total concentration (M).
 * Temperature models take x = temperature (K) and use the van 't Hoff forms
 *   K     = exp(-deltaH / RT + deltaS / R)
 *   sigma = exp(-deltaHnuc / RT)          (deltaHnuc > 0 is a nucleation penalty)
 * Note: the older model_fitting.fitting used sigma = exp(+deltaHnuc / RT), i.e.
 * the opposite sign of deltaHnuc.
 *
 * `c_tot` is fixed by default and follows the dataset constant `conc_M` (read
 * from file names like `..._50microM_...`) when the dataset has one.
 *
 * An ordinary fit_it C plugin -- see peaks.c for how presets relate to plugins.
 */
#include "fit_it_plugin.h"

#include <math.h>

#define R_GAS 8.314
#define INF HUGE_VAL
#define N_ITER 100

/* ---- inverse models: total concentration from free monomer concentration ---- */

/* Returns INFINITY past the singularity (K c >= 1) so bisection brackets correctly. */
static double inv_isodesmic(double cm, double k)
{
    double d = 1.0 - k * cm;
    if (d <= 0.0)
        return INF;
    return cm / (d * d);
}

static double inv_cooperative(double cm, double k, double sigma)
{
    double ck, d;
    if (k == 0.0)
        return cm;
    ck = k * cm;
    if (ck >= 1.0)
        return INF;
    d = 1.0 - ck;
    return cm + sigma / k * (ck * ck * (2.0 - ck)) / (d * d);
}

/* Nucleus size N >= 2: species of size s carry sigma^(min(s, N) - 1). */
static double inv_cooperative_n(double cm, double k, double sigma, int n)
{
    double ck, d, elong, ln_sigma, corr = 0.0, ck_pow, sigma_pow;
    int s;
    if (k == 0.0)
        return cm;
    ck = k * cm;
    if (ck >= 1.0)
        return INF;
    d = 1.0 - ck;
    elong = pow(sigma, n - 1) / k * (ck * ck * (2.0 - ck)) / (d * d);
    /* 1 - sigma^m evaluated without cancellation near sigma = 1. */
    ln_sigma = log1p(sigma - 1.0);
    ck_pow = ck * ck;
    sigma_pow = sigma;
    for (s = 2; s < n; ++s) {
        double one_minus = -expm1((double)(n - s) * ln_sigma);
        corr += (double)s * sigma_pow * one_minus * ck_pow;
        ck_pow *= ck;
        sigma_pow *= sigma;
    }
    return cm + elong + corr / k;
}

enum kind { ISO, COOP, COOP_N, COOP_ISO };

static double inverse(enum kind kind, double cm, const double *k)
{
    switch (kind) {
    case ISO:
        return inv_isodesmic(cm, k[0]);
    case COOP:
        return inv_cooperative(cm, k[0], k[1]);
    case COOP_N:
        return inv_cooperative_n(cm, k[0], k[1], (int)k[2]);
    case COOP_ISO: /* shared monomer: iso + coop - monomer (counted once) */
        return inv_isodesmic(cm, k[0]) + inv_cooperative(cm, k[1], k[2]) - cm;
    }
    return NAN;
}

/* Aggregated fraction 1 - c_monomer / c_tot, by bisection on c_monomer in (0, 1/K). */
static double aggregate(enum kind kind, double conc, const double *k)
{
    double lo = 0.0, hi, kmax;
    int i;
    if (!(conc > 0.0))
        return 0.0;
    kmax = kind == COOP_ISO ? fmax(k[0], k[1]) : k[0];
    if (!(kmax > 0.0))
        return 0.0; /* no association at all */
    hi = 1.0 / kmax;
    for (i = 0; i < N_ITER; ++i) {
        double mid = 0.5 * (lo + hi);
        if (inverse(kind, mid, k) - conc <= 0.0)
            lo = mid;
        else
            hi = mid;
    }
    return 1.0 - 0.5 * (lo + hi) / conc;
}

static double van_t_hoff(double t, double dh, double ds)
{
    return exp(-dh / (R_GAS * t) + ds / R_GAS);
}

static double penalty(double t, double dh_nuc)
{
    return exp(-dh_nuc / (R_GAS * t));
}

static int nucleus(double v)
{
    int n = (int)floor(v + 0.5);
    return n < 2 ? 2 : n;
}

/* ---- concentration models (x = total concentration, M) ---- */

static int32_t isodesmic(const double *x, size_t n, const double *p, double *out)
{
    for (size_t i = 0; i < n; ++i) {
        /* Closed form: with b = K c, K c_mono = 2b / (2b + 1 + sqrt(4b + 1)). */
        double b = p[0] * x[i];
        out[i] = b > 0.0 ? p[1] * (1.0 - 2.0 / (2.0 * b + 1.0 + sqrt(4.0 * b + 1.0))) : 0.0;
    }
    return 0;
}

static int32_t cooperative(const double *x, size_t n, const double *p, double *out)
{
    double k[2] = {p[0], p[1]};
    for (size_t i = 0; i < n; ++i)
        out[i] = p[2] * aggregate(COOP, x[i], k);
    return 0;
}

static int32_t cooperative_n(const double *x, size_t n, const double *p, double *out)
{
    double k[3] = {p[0], p[1], (double)nucleus(p[2])};
    for (size_t i = 0; i < n; ++i)
        out[i] = p[3] * aggregate(COOP_N, x[i], k);
    return 0;
}

static int32_t coop_iso(const double *x, size_t n, const double *p, double *out)
{
    double k[3] = {p[0], p[1], p[2]};
    for (size_t i = 0; i < n; ++i)
        out[i] = p[3] * aggregate(COOP_ISO, x[i], k);
    return 0;
}

/* ---- temperature models (x = temperature, K) ---- */

static int32_t temp_isodesmic(const double *x, size_t n, const double *p, double *out)
{
    /* deltaH, deltaS, c_tot, scaler */
    for (size_t i = 0; i < n; ++i) {
        double b = van_t_hoff(x[i], p[0], p[1]) * p[2];
        out[i] = b > 0.0 ? p[3] * (1.0 - 2.0 / (2.0 * b + 1.0 + sqrt(4.0 * b + 1.0))) : 0.0;
    }
    return 0;
}

static int32_t temp_cooperative(const double *x, size_t n, const double *p, double *out)
{
    /* deltaH, deltaS, deltaHnuc, c_tot, scaler */
    for (size_t i = 0; i < n; ++i) {
        double k[2] = {van_t_hoff(x[i], p[0], p[1]), penalty(x[i], p[2])};
        out[i] = p[4] * aggregate(COOP, p[3], k);
    }
    return 0;
}

static int32_t temp_cooperative_n(const double *x, size_t n, const double *p, double *out)
{
    /* deltaH, deltaS, deltaHnuc, nuc_size, c_tot, scaler */
    for (size_t i = 0; i < n; ++i) {
        double k[3] = {van_t_hoff(x[i], p[0], p[1]), penalty(x[i], p[2]), (double)nucleus(p[3])};
        out[i] = p[5] * aggregate(COOP_N, p[4], k);
    }
    return 0;
}

static int32_t temp_coop_iso(const double *x, size_t n, const double *p, double *out)
{
    /* deltaH_iso, deltaS_iso, deltaH_coop, deltaS_coop, deltaHnuc_coop, c_tot, scaler */
    for (size_t i = 0; i < n; ++i) {
        double k[3] = {van_t_hoff(x[i], p[0], p[1]), van_t_hoff(x[i], p[2], p[3]), penalty(x[i], p[4])};
        out[i] = p[6] * aggregate(COOP_ISO, p[5], k);
    }
    return 0;
}

/* ---- parameter tables ---- */

#define P_SCALER {"scaler", "", "signal of the fully aggregated state", 1.0, 0.0, INF}
#define P_CTOT {"c_tot", "M", "total concentration (fixed; follows the dataset constant conc_M)", 1e-5, 0.0, INF, FIT_IT_PARAM_FIXED, "conc_M"}
#define P_DH(name) {name, "J/mol", "elongation enthalpy", -100000.0, -INF, INF}
#define P_DS(name) {name, "J/(mol K)", "elongation entropy", -200.0, -INF, INF}
#define P_DHNUC(name) {name, "J/mol", "nucleation penalty (sigma = exp(-deltaHnuc/RT))", 10000.0, -INF, INF}
#define P_NUC {"nuc_size", "", "nucleus size N (rounded, >= 2)", 3.0, 2.0, 50.0, FIT_IT_PARAM_FIXED}

static const FitItParam isodesmic_p[] = {
    {"K", "1/M", "association constant", 1e5, 0.0, INF},
    P_SCALER,
};
static const FitItParam cooperative_p[] = {
    {"K", "1/M", "elongation constant", 1e5, 0.0, INF},
    {"sigma", "", "cooperativity (K_nuc = sigma K)", 0.01, 0.0, 1.0},
    P_SCALER,
};
static const FitItParam cooperative_n_p[] = {
    {"K", "1/M", "elongation constant", 1e5, 0.0, INF},
    {"sigma", "", "cooperativity", 0.01, 0.0, 1.0},
    P_NUC,
    P_SCALER,
};
static const FitItParam coop_iso_p[] = {
    {"K_iso", "1/M", "isodesmic pathway constant", 1e4, 0.0, INF},
    {"K_coop", "1/M", "cooperative pathway elongation constant", 1e5, 0.0, INF},
    {"sigma", "", "cooperativity of the cooperative pathway", 0.01, 0.0, 1.0},
    P_SCALER,
};
static const FitItParam temp_isodesmic_p[] = {P_DH("deltaH"), P_DS("deltaS"), P_CTOT, P_SCALER};
static const FitItParam temp_cooperative_p[] = {P_DH("deltaH"), P_DS("deltaS"), P_DHNUC("deltaHnuc"), P_CTOT, P_SCALER};
static const FitItParam temp_cooperative_n_p[] = {
    P_DH("deltaH"), P_DS("deltaS"), P_DHNUC("deltaHnuc"), P_NUC, P_CTOT, P_SCALER,
};
static const FitItParam temp_coop_iso_p[] = {
    P_DH("deltaH_iso"), P_DS("deltaS_iso"), P_DH("deltaH_coop"), P_DS("deltaS_coop"),
    P_DHNUC("deltaHnuc_coop"), P_CTOT, P_SCALER,
};

#define COUNT(a) ((uint32_t)(sizeof(a) / sizeof((a)[0])))
#define CAT "Supramolecular"

static const FitItModel models[] = {
    {FIT_IT_ABI_VERSION, COUNT(isodesmic_p), "Isodesmic", CAT,
     "Aggregated fraction vs total concentration x (M), isodesmic (equal-K) growth", isodesmic_p, isodesmic, NULL},
    {FIT_IT_ABI_VERSION, COUNT(cooperative_p), "Cooperative", CAT,
     "Aggregated fraction vs total concentration x (M), nucleation-elongation with dimer nucleus",
     cooperative_p, cooperative, NULL},
    {FIT_IT_ABI_VERSION, COUNT(cooperative_n_p), "CooperativeN", CAT,
     "Aggregated fraction vs total concentration x (M), nucleation-elongation with nucleus size N",
     cooperative_n_p, cooperative_n, NULL},
    {FIT_IT_ABI_VERSION, COUNT(coop_iso_p), "CoopIso", CAT,
     "Aggregated fraction vs total concentration x (M), competing isodesmic and cooperative pathways",
     coop_iso_p, coop_iso, NULL},
    {FIT_IT_ABI_VERSION, COUNT(temp_isodesmic_p), "TempIsodesmic", CAT,
     "Aggregated fraction vs temperature x (K), isodesmic, K from deltaH/deltaS", temp_isodesmic_p, temp_isodesmic, NULL},
    {FIT_IT_ABI_VERSION, COUNT(temp_cooperative_p), "TempCooperative", CAT,
     "Aggregated fraction vs temperature x (K), cooperative: K from deltaH/deltaS, sigma = exp(-deltaHnuc/RT)",
     temp_cooperative_p, temp_cooperative, NULL},
    {FIT_IT_ABI_VERSION, COUNT(temp_cooperative_n_p), "TempCooperativeN", CAT,
     "Aggregated fraction vs temperature x (K), cooperative with nucleus size N", temp_cooperative_n_p,
     temp_cooperative_n, NULL},
    {FIT_IT_ABI_VERSION, COUNT(temp_coop_iso_p), "TempCoopIso", CAT,
     "Aggregated fraction vs temperature x (K), competing isodesmic and cooperative pathways",
     temp_coop_iso_p, temp_coop_iso, NULL},
};

FIT_IT_EXPORT const FitItModel *fit_it_models(uint32_t *count)
{
    *count = COUNT(models);
    return models;
}
