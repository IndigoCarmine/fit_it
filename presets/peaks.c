/*
 * fit_it preset: peak shapes (lmfit parameterisation: `amplitude` is the area).
 *
 * This is an ordinary fit_it C plugin -- the same format as files in your
 * plugin folder. Copy it there and edit it to make your own variant; a model
 * with the same name in the plugin folder replaces the preset.
 */
#include "fit_it_plugin.h"

#include <math.h>
#include <stdlib.h>

#define PI 3.14159265358979323846
#define SQRT_2PI 2.5066282746310002
#define LN2 0.69314718055994530942
#define INF HUGE_VAL

/* Point-wise eval function from an expression in x and p[]. */
#define POINTWISE(fname, expr)                                                  \
    static int32_t fname(const double *xs, size_t n, const double *p, double *out) \
    {                                                                           \
        for (size_t i = 0; i < n; ++i) {                                        \
            const double x = xs[i];                                             \
            out[i] = (expr);                                                    \
        }                                                                       \
        return 0;                                                               \
    }

static double gauss(double x, double c, double s)
{
    return exp(-(x - c) * (x - c) / (2.0 * s * s)) / (s * SQRT_2PI);
}

static double lorentz(double x, double c, double s)
{
    return s / (PI * ((x - c) * (x - c) + s * s));
}

/* ---- minimal complex arithmetic (MSVC has no C99 complex) ---- */
typedef struct { double re, im; } cplx;
static cplx c_(double re, double im) { cplx z = {re, im}; return z; }
static cplx cadd(cplx a, cplx b) { return c_(a.re + b.re, a.im + b.im); }
static cplx csub(cplx a, cplx b) { return c_(a.re - b.re, a.im - b.im); }
static cplx cmul(cplx a, cplx b) { return c_(a.re * b.re - a.im * b.im, a.re * b.im + a.im * b.re); }
static cplx cdiv(cplx a, cplx b)
{
    double d = b.re * b.re + b.im * b.im;
    return c_((a.re * b.re + a.im * b.im) / d, (a.im * b.re - a.re * b.im) / d);
}
static cplx r_(double v) { return c_(v, 0.0); }

/* Re w(x + iy), y >= 0 -- Humlicek (1982) W4, relative error < 1e-4. */
static double faddeeva_re(double x, double y)
{
    cplx t = c_(y, -x), w;
    double s = fabs(x) + y;
    if (s >= 15.0) {
        w = cdiv(cmul(t, r_(0.5641896)), cadd(r_(0.5), cmul(t, t)));
    } else if (s >= 5.5) {
        cplx u = cmul(t, t);
        w = cdiv(cmul(t, cadd(r_(1.410474), cmul(u, r_(0.5641896)))),
                 cadd(r_(0.75), cmul(u, cadd(r_(3.0), u))));
    } else if (y >= 0.195 * fabs(x) - 0.176) {
        static const double a[] = {16.4955, 20.20933, 11.96482, 3.778987, 0.5642236};
        static const double b[] = {16.4955, 38.82363, 39.27121, 21.69274, 6.699398};
        cplx num = r_(a[4]), den = cadd(r_(b[4]), t);
        for (int k = 3; k >= 0; --k) {
            num = cadd(r_(a[k]), cmul(t, num));
            den = cadd(r_(b[k]), cmul(t, den));
        }
        w = cdiv(num, den);
    } else {
        static const double a[] = {36183.31, 3321.9905, 1540.787, 219.0313, 35.76683, 1.320522, 0.56419};
        static const double b[] = {32066.6, 24322.84, 9022.228, 2186.181, 364.2191, 61.57037, 1.841439};
        cplx u = cmul(t, t);
        double e = exp(u.re);
        cplx expu = c_(e * cos(u.im), e * sin(u.im));
        cplx num = r_(a[6]), den = csub(r_(b[6]), u);
        for (int k = 5; k >= 0; --k) {
            num = csub(r_(a[k]), cmul(u, num));
            den = csub(r_(b[k]), cmul(u, den));
        }
        w = csub(expu, cdiv(cmul(t, num), den));
    }
    return w.re;
}

static double voigt(double x, double c, double s, double g)
{
    double k = s * sqrt(2.0);
    return faddeeva_re((x - c) / k, g / k) / (s * SQRT_2PI);
}

/* ---- initial guesses ---- */
typedef struct { double x, y; } pt;

static int cmp_pt(const void *a, const void *b)
{
    double d = ((const pt *)a)->x - ((const pt *)b)->x;
    return (d > 0) - (d < 0);
}

/* Height above baseline, center and FWHM of the tallest feature. */
static int peak_estimate(const double *x, const double *y, size_t n, double *height, double *center, double *fwhm)
{
    pt *v = (pt *)malloc(n * sizeof(pt));
    size_t m = 0, imax = 0, i;
    double base = INF, half, left, right;
    if (!v)
        return 1;
    for (i = 0; i < n; ++i)
        if (isfinite(x[i]) && isfinite(y[i])) {
            v[m].x = x[i];
            v[m].y = y[i];
            ++m;
        }
    if (m < 3) {
        free(v);
        return 1;
    }
    qsort(v, m, sizeof(pt), cmp_pt);
    for (i = 0; i < m; ++i) {
        if (v[i].y > v[imax].y)
            imax = i;
        if (v[i].y < base)
            base = v[i].y;
    }
    *height = v[imax].y - base;
    *center = v[imax].x;
    half = base + *height / 2.0;
    left = v[0].x;
    for (i = imax; i-- > 0;)
        if (v[i].y < half) {
            left = v[i].x;
            break;
        }
    right = v[m - 1].x;
    for (i = imax; i < m; ++i)
        if (v[i].y < half) {
            right = v[i].x;
            break;
        }
    *fwhm = right - left;
    if (!(*fwhm > 0.0))
        *fwhm = (v[m - 1].x - v[0].x) / 10.0;
    free(v);
    return 0;
}

#define FWHM_TO_SIGMA (1.0 / (2.0 * sqrt(2.0 * LN2)))

/* ---- Gaussian ---- */
static const FitItParam gaussian_p[] = {
    {"amplitude", "", "area", 1.0, -INF, INF},
    {"center", "", "", 0.0, -INF, INF},
    {"sigma", "", "FWHM = 2.3548 sigma", 1.0, 0.0, INF},
};
POINTWISE(gaussian, p[0] * gauss(x, p[1], p[2]))
static int32_t gaussian_guess(const double *x, const double *y, size_t n, double *p)
{
    double h, c, w;
    if (peak_estimate(x, y, n, &h, &c, &w))
        return 1;
    p[2] = w * FWHM_TO_SIGMA;
    p[0] = h * p[2] * SQRT_2PI;
    p[1] = c;
    return 0;
}

/* ---- Lorentzian ---- */
static const FitItParam lorentzian_p[] = {
    {"amplitude", "", "area", 1.0, -INF, INF},
    {"center", "", "", 0.0, -INF, INF},
    {"sigma", "", "half width at half maximum", 1.0, 0.0, INF},
};
POINTWISE(lorentzian, p[0] * lorentz(x, p[1], p[2]))
static int32_t lorentzian_guess(const double *x, const double *y, size_t n, double *p)
{
    double h, c, w;
    if (peak_estimate(x, y, n, &h, &c, &w))
        return 1;
    p[2] = w / 2.0;
    p[0] = h * PI * p[2];
    p[1] = c;
    return 0;
}

/* ---- Pseudo-Voigt ---- */
static const FitItParam pseudo_voigt_p[] = {
    {"amplitude", "", "area", 1.0, -INF, INF},
    {"center", "", "", 0.0, -INF, INF},
    {"sigma", "", "half width at half maximum", 1.0, 0.0, INF},
    {"fraction", "", "Lorentzian fraction", 0.5, 0.0, 1.0},
};
POINTWISE(pseudo_voigt, p[0] * ((1.0 - p[3]) * gauss(x, p[1], p[2] / sqrt(2.0 * LN2)) + p[3] * lorentz(x, p[1], p[2])))
static int32_t pseudo_voigt_guess(const double *x, const double *y, size_t n, double *p)
{
    double h, c, w, s, peak;
    if (peak_estimate(x, y, n, &h, &c, &w))
        return 1;
    s = w / 2.0;
    peak = 0.5 * gauss(0.0, 0.0, s / sqrt(2.0 * LN2)) + 0.5 * lorentz(0.0, 0.0, s);
    p[0] = h / peak;
    p[1] = c;
    p[2] = s;
    p[3] = 0.5;
    return 0;
}

/* ---- Voigt ---- */
static const FitItParam voigt_p[] = {
    {"amplitude", "", "area", 1.0, -INF, INF},
    {"center", "", "", 0.0, -INF, INF},
    {"sigma", "", "Gaussian width", 1.0, 0.0, INF},
    {"gamma", "", "Lorentzian half width", 1.0, 0.0, INF},
};
POINTWISE(voigt_eval, p[0] * voigt(x, p[1], p[2], p[3]))
static int32_t voigt_guess(const double *x, const double *y, size_t n, double *p)
{
    double h, c, w, s;
    if (peak_estimate(x, y, n, &h, &c, &w))
        return 1;
    s = w / 3.6;
    p[0] = h / voigt(0.0, 0.0, s, s);
    p[1] = c;
    p[2] = s;
    p[3] = s;
    return 0;
}

/* ---- Pearson VII ---- */
static const FitItParam pearson7_p[] = {
    {"height", "", "", 1.0, -INF, INF},
    {"center", "", "", 0.0, -INF, INF},
    {"hwhm", "", "half width at half maximum", 1.0, 0.0, INF},
    {"expon", "", "1 = Lorentzian, large = Gaussian", 1.5, 0.5, INF},
};
POINTWISE(pearson7, p[0] * pow(1.0 + ((x - p[1]) / p[2]) * ((x - p[1]) / p[2]) * (pow(2.0, 1.0 / p[3]) - 1.0), -p[3]))
static int32_t pearson7_guess(const double *x, const double *y, size_t n, double *p)
{
    double h, c, w;
    if (peak_estimate(x, y, n, &h, &c, &w))
        return 1;
    p[0] = h;
    p[1] = c;
    p[2] = w / 2.0;
    p[3] = 1.5;
    return 0;
}

/* ---- Skewed Gaussian ---- */
static const FitItParam skewed_gaussian_p[] = {
    {"amplitude", "", "area", 1.0, -INF, INF},
    {"center", "", "", 0.0, -INF, INF},
    {"sigma", "", "", 1.0, 0.0, INF},
    {"gamma", "", "skewness", 0.0, -INF, INF},
};
POINTWISE(skewed_gaussian, p[0] * gauss(x, p[1], p[2]) * (1.0 + erf(p[3] * (x - p[1]) / (p[2] * sqrt(2.0)))))
static int32_t skewed_gaussian_guess(const double *x, const double *y, size_t n, double *p)
{
    if (gaussian_guess(x, y, n, p))
        return 1;
    p[3] = 0.0;
    return 0;
}

#define COUNT(a) ((uint32_t)(sizeof(a) / sizeof((a)[0])))

static const FitItModel models[] = {
    {FIT_IT_ABI_VERSION, COUNT(gaussian_p), "Gaussian", "Peak",
     "amplitude/(sigma*sqrt(2pi)) * exp(-(x-center)^2 / (2 sigma^2))", gaussian_p, gaussian, gaussian_guess},
    {FIT_IT_ABI_VERSION, COUNT(lorentzian_p), "Lorentzian", "Peak",
     "amplitude/pi * sigma / ((x-center)^2 + sigma^2)", lorentzian_p, lorentzian, lorentzian_guess},
    {FIT_IT_ABI_VERSION, COUNT(pseudo_voigt_p), "PseudoVoigt", "Peak",
     "(1-fraction) Gaussian + fraction Lorentzian, both with FWHM = 2 sigma", pseudo_voigt_p, pseudo_voigt, pseudo_voigt_guess},
    {FIT_IT_ABI_VERSION, COUNT(voigt_p), "Voigt", "Peak",
     "Gaussian (sigma) convolved with Lorentzian (gamma), via the Faddeeva function", voigt_p, voigt_eval, voigt_guess},
    {FIT_IT_ABI_VERSION, COUNT(pearson7_p), "PearsonVII", "Peak",
     "height * (1 + ((x-center)/hwhm)^2 (2^(1/expon)-1))^(-expon)", pearson7_p, pearson7, pearson7_guess},
    {FIT_IT_ABI_VERSION, COUNT(skewed_gaussian_p), "SkewedGaussian", "Peak",
     "Gaussian * (1 + erf(gamma (x-center) / (sigma sqrt 2)))", skewed_gaussian_p, skewed_gaussian, skewed_gaussian_guess},
};

FIT_IT_EXPORT const FitItModel *fit_it_models(uint32_t *count)
{
    *count = COUNT(models);
    return models;
}
