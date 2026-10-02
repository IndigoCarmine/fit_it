/*
 * fit_it preset: supramolecular polymerization models.
 *
 * A port of sp_fitting_models (https://github.com/IndigoCarmine/sp_fitting_models,
 * src/lib.rs, v1.3.9): the same mass balances, solved for the free monomer by a
 * bracketed bisection (at most 100 steps), with R = 8.314 J/(mol K). Every model
 * returns the aggregated fraction (clamped to [0, 1]) times `scaler`.
 *
 * As upstream, a non-positive or non-finite concentration / temperature / c_tot is an
 * error (eval returns 1), and a negative K or sigma gives NaN.
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

/* +INFINITY past the singularity (K c >= 1) so bisection brackets correctly; NaN stays NaN. */
static double past_singularity(double ck)
{
    return isnan(ck) ? NAN : INF;
}

static double inv_isodesmic(double cm, double k)
{
    double ck = k * cm, d;
    if (!(ck < 1.0))
        return past_singularity(ck);
    d = 1.0 - ck;
    return cm / (d * d);
}

static double inv_cooperative(double cm, double k, double sigma)
{
    double ck, d;
    if (k == 0.0)
        return cm;
    ck = k * cm;
    if (!(ck < 1.0))
        return past_singularity(ck);
    d = 1.0 - ck;
    return cm + sigma * ck * cm * (2.0 - ck) / (d * d);
}

/* Nucleus size N >= 2: species of size s carry sigma^(min(s, N) - 1). */
static double inv_cooperative_n(double cm, double k, double sigma, int n)
{
    double ck, d, elong, ln_sigma, corr = 0.0, ck_pow, sigma_pow;
    int s;
    if (k == 0.0)
        return cm;
    ck = k * cm;
    if (!(ck < 1.0))
        return past_singularity(ck);
    d = 1.0 - ck;
    elong = pow(sigma, n - 1) * ck * cm * (2.0 - ck) / (d * d);
    /* 1 - sigma^m evaluated without cancellation near sigma = 1. */
    ln_sigma = log1p(sigma - 1.0);
    ck_pow = ck; /* (K c)^(s-1); times c below this gives K^(s-1) c^s */
    sigma_pow = sigma;
    for (s = 2; s < n; ++s) {
        double one_minus = -expm1((double)(n - s) * ln_sigma);
        corr += (double)s * sigma_pow * one_minus * ck_pow;
        ck_pow *= ck;
        sigma_pow *= sigma;
    }
    return cm + elong + corr * cm;
}

enum kind { COOP, COOP_N, COOP_ISO };

static double inverse(enum kind kind, double cm, const double *k)
{
    switch (kind) {
    case COOP:
        return inv_cooperative(cm, k[0], k[1]);
    case COOP_N:
        return inv_cooperative_n(cm, k[0], k[1], (int)k[2]);
    case COOP_ISO: /* shared monomer: iso + coop - monomer (counted once) */
        return inv_isodesmic(cm, k[0]) + inv_cooperative(cm, k[1], k[2]) - cm;
    }
    return NAN;
}

/*
 * Aggregated fraction 1 - c_monomer / conc, by bisection on c_monomer.
 * The root lies in [0, min(conc, x_max)] (x_max = singularity of the inverse model,
 * e.g. 1/K), so a tiny K cannot leave the bisection unconverged. Non-finite inverse
 * values count as "above the root"; iteration stops once the interval can no longer
 * shrink; the result is clamped to [0, 1].
 */
static double aggregate(enum kind kind, double conc, double x_max, const double *k)
{
    double lo = 0.0, hi;
    int i;
    if (!(conc > 0.0) || isnan(x_max))
        return NAN;
    hi = fmax(fmin(conc, x_max), 0.0);
    if (hi == 0.0)
        return 1.0; /* K -> infinity: no free monomer remains */
    for (i = 0; i < N_ITER; ++i) {
        double mid = 0.5 * (lo + hi), f;
        if (mid <= lo || mid >= hi)
            break;
        f = inverse(kind, mid, k);
        if (isfinite(f) && f <= conc)
            lo = mid;
        else
            hi = mid;
    }
    return fmin(fmax(1.0 - 0.5 * (lo + hi) / conc, 0.0), 1.0);
}

/* Closed-form isodesmic aggregation for conc > 0, cancellation-free for small K c. */
static double isodesmic_direct(double conc, double k)
{
    double b, s, den;
    if (!(k >= 0.0))
        return NAN;
    b = k * conc;
    if (b == 0.0)
        return 0.0;
    s = sqrt(4.0 * b + 1.0);
    den = 2.0 * b + 1.0 + s;
    if (!isfinite(den))
        return 1.0;
    return fmin(fmax((2.0 * b + 4.0 * b / (s + 1.0)) / den, 0.0), 1.0);
}

static double coop(double conc, double k, double sigma)
{
    double kk[2] = {k, sigma};
    if (!(k >= 0.0) || !(sigma >= 0.0))
        return NAN;
    return aggregate(COOP, conc, 1.0 / k, kk);
}

static double coop_n(double conc, double k, double sigma, int n)
{
    double kk[3] = {k, sigma, (double)n};
    if (!(k >= 0.0) || !(sigma >= 0.0))
        return NAN;
    return aggregate(COOP_N, conc, 1.0 / k, kk);
}

static double coop_iso_agg(double conc, double k_iso, double k_coop, double sigma)
{
    double kk[3] = {k_iso, k_coop, sigma};
    if (!(k_iso >= 0.0) || !(k_coop >= 0.0) || !(sigma >= 0.0))
        return NAN;
    return aggregate(COOP_ISO, conc, fmin(1.0 / k_iso, 1.0 / k_coop), kk);
}

/* Overflow to inf / underflow to 0 is fine: the solvers handle both limits. */
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

/* Concentrations and temperatures must be positive and finite. */
static int positive(double v)
{
    return isfinite(v) && v > 0.0;
}

/* ---- concentration models (x = total concentration, M) ---- */

static int32_t isodesmic(const double *x, size_t n, const double *p, double *out)
{
    for (size_t i = 0; i < n; ++i) {
        if (!positive(x[i]))
            return 1;
        out[i] = p[1] * isodesmic_direct(x[i], p[0]);
    }
    return 0;
}

static int32_t cooperative(const double *x, size_t n, const double *p, double *out)
{
    for (size_t i = 0; i < n; ++i) {
        if (!positive(x[i]))
            return 1;
        out[i] = p[2] * coop(x[i], p[0], p[1]);
    }
    return 0;
}

static int32_t cooperative_n(const double *x, size_t n, const double *p, double *out)
{
    int nuc = nucleus(p[2]);
    for (size_t i = 0; i < n; ++i) {
        if (!positive(x[i]))
            return 1;
        out[i] = p[3] * coop_n(x[i], p[0], p[1], nuc);
    }
    return 0;
}

static int32_t coop_iso(const double *x, size_t n, const double *p, double *out)
{
    for (size_t i = 0; i < n; ++i) {
        if (!positive(x[i]))
            return 1;
        out[i] = p[3] * coop_iso_agg(x[i], p[0], p[1], p[2]);
    }
    return 0;
}

/* ---- temperature models (x = temperature, K) ---- */

static int32_t temp_isodesmic(const double *x, size_t n, const double *p, double *out)
{
    /* deltaH, deltaS, c_tot, scaler */
    if (!positive(p[2]))
        return 1;
    for (size_t i = 0; i < n; ++i) {
        if (!positive(x[i]))
            return 1;
        out[i] = p[3] * isodesmic_direct(p[2], van_t_hoff(x[i], p[0], p[1]));
    }
    return 0;
}

static int32_t temp_cooperative(const double *x, size_t n, const double *p, double *out)
{
    /* deltaH, deltaS, deltaHnuc, c_tot, scaler */
    if (!positive(p[3]))
        return 1;
    for (size_t i = 0; i < n; ++i) {
        if (!positive(x[i]))
            return 1;
        out[i] = p[4] * coop(p[3], van_t_hoff(x[i], p[0], p[1]), penalty(x[i], p[2]));
    }
    return 0;
}

static int32_t temp_cooperative_n(const double *x, size_t n, const double *p, double *out)
{
    /* deltaH, deltaS, deltaHnuc, nuc_size, c_tot, scaler */
    int nuc = nucleus(p[3]);
    if (!positive(p[4]))
        return 1;
    for (size_t i = 0; i < n; ++i) {
        if (!positive(x[i]))
            return 1;
        out[i] = p[5] * coop_n(p[4], van_t_hoff(x[i], p[0], p[1]), penalty(x[i], p[2]), nuc);
    }
    return 0;
}

static int32_t temp_coop_iso(const double *x, size_t n, const double *p, double *out)
{
    /* deltaH_iso, deltaS_iso, deltaH_coop, deltaS_coop, deltaHnuc_coop, c_tot, scaler */
    if (!positive(p[5]))
        return 1;
    for (size_t i = 0; i < n; ++i) {
        if (!positive(x[i]))
            return 1;
        out[i] = p[6] * coop_iso_agg(p[5], van_t_hoff(x[i], p[0], p[1]), van_t_hoff(x[i], p[2], p[3]),
                                     penalty(x[i], p[4]));
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
