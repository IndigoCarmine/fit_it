/*
 * fit_it preset: small-angle scattering models (SasView parameterisation).
 *
 * x is q in 1/Angstrom, SLDs are in 1e-6/Angstrom^2, lengths in Angstrom and
 * intensities in 1/cm, as in SasView. `scale` and `background` are explicit
 * parameters here (SasView adds them implicitly).
 *
 * An ordinary fit_it C plugin -- see peaks.c for how presets relate to plugins.
 */
#include "fit_it_plugin.h"

#include <math.h>

#define PI 3.14159265358979323846
#define INF HUGE_VAL

#define POINTWISE(fname, expr)                                                  \
    static int32_t fname(const double *xs, size_t n, const double *p, double *out) \
    {                                                                           \
        for (size_t i = 0; i < n; ++i) {                                        \
            const double x = xs[i];                                             \
            out[i] = (expr);                                                    \
        }                                                                       \
        return 0;                                                               \
    }

/* ---- Guinier ---- */
static const FitItParam guinier_p[] = {
    {"scale", "", "I(0)", 1.0, 0.0, INF},
    {"background", "1/cm", "", 0.0, -INF, INF},
    {"rg", "A", "radius of gyration", 60.0, 0.0, INF},
};
POINTWISE(guinier, p[0] * exp(-x * x * p[2] * p[2] / 3.0) + p[1])

/* ---- Porod ---- */
static const FitItParam porod_p[] = {
    {"scale", "", "", 1e-6, -INF, INF},
    {"background", "1/cm", "", 0.0, -INF, INF},
};
POINTWISE(porod, p[0] * pow(x, -4.0) + p[1])

/* ---- Sphere ---- */
static const FitItParam sphere_p[] = {
    {"scale", "", "volume fraction", 1.0, 0.0, INF},
    {"background", "1/cm", "", 0.001, -INF, INF},
    {"sld", "1e-6/A^2", "particle SLD", 1.0, -INF, INF},
    {"sld_solvent", "1e-6/A^2", "solvent SLD", 6.0, -INF, INF},
    {"radius", "A", "", 50.0, 0.0, INF},
};
static double sphere_amp(double qr)
{
    if (qr < 1e-3)
        return 1.0 - qr * qr / 10.0;
    return 3.0 * (sin(qr) - qr * cos(qr)) / (qr * qr * qr);
}
static int32_t sphere(const double *xs, size_t n, const double *p, double *out)
{
    double r = p[4], v = 4.0 / 3.0 * PI * r * r * r, drho = p[2] - p[3];
    for (size_t i = 0; i < n; ++i) {
        double f = sphere_amp(xs[i] * r);
        out[i] = 1e-4 * p[0] * v * drho * drho * f * f + p[1];
    }
    return 0;
}

/* ---- Debye (monodisperse Gaussian coil) ---- */
static const FitItParam debye_p[] = {
    {"scale", "", "", 1.0, 0.0, INF},
    {"background", "1/cm", "", 0.001, -INF, INF},
    {"i_zero", "1/cm", "I(0)", 70.0, 0.0, INF},
    {"rg", "A", "radius of gyration", 75.0, 0.0, INF},
};
static double debye_fn(double u)
{
    if (u < 1e-4)
        return 1.0 - u / 3.0;
    return 2.0 * (exp(-u) + u - 1.0) / (u * u);
}
POINTWISE(debye, p[0] * p[2] * debye_fn(x * x * p[3] * p[3]) + p[1])

#define COUNT(a) ((uint32_t)(sizeof(a) / sizeof((a)[0])))

static const FitItModel models[] = {
    {FIT_IT_ABI_VERSION, COUNT(guinier_p), "Guinier", "SAS", "scale * exp(-q^2 rg^2 / 3) + background", guinier_p, guinier, NULL},
    {FIT_IT_ABI_VERSION, COUNT(porod_p), "Porod", "SAS", "scale * q^-4 + background", porod_p, porod, NULL},
    {FIT_IT_ABI_VERSION, COUNT(sphere_p), "Sphere", "SAS",
     "Homogeneous sphere form factor: scale * V (sld - sld_solvent)^2 [3 (sin qr - qr cos qr) / (qr)^3]^2 * 1e-4 + background",
     sphere_p, sphere, NULL},
    {FIT_IT_ABI_VERSION, COUNT(debye_p), "Debye", "SAS",
     "Gaussian coil: scale * i_zero * 2 (exp(-u) + u - 1) / u^2 + background, u = (q rg)^2", debye_p, debye, NULL},
};

FIT_IT_EXPORT const FitItModel *fit_it_models(uint32_t *count)
{
    *count = COUNT(models);
    return models;
}
