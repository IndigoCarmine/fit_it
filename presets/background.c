/*
 * fit_it preset: backgrounds, polynomials, decays and steps.
 *
 * An ordinary fit_it C plugin -- see peaks.c for how presets relate to plugins.
 */
#include "fit_it_plugin.h"

#include <math.h>

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

/* Least-squares polynomial c[0] + c[1] u + ... of degree deg (<= 3) through (u, v). */
static int polyfit(const double *u, const double *v, size_t n, int deg, double *c)
{
    double a[4][5] = {{0}};
    int m = deg + 1, i, j, k;
    size_t r;
    for (r = 0; r < n; ++r) {
        double pw[7], t = 1.0;
        if (!isfinite(u[r]) || !isfinite(v[r]))
            continue;
        for (i = 0; i < 2 * m - 1; ++i) {
            pw[i] = t;
            t *= u[r];
        }
        for (i = 0; i < m; ++i) {
            for (j = 0; j < m; ++j)
                a[i][j] += pw[i + j];
            a[i][m] += pw[i] * v[r];
        }
    }
    for (i = 0; i < m; ++i) { /* Gauss-Jordan with partial pivoting */
        int piv = i;
        for (k = i + 1; k < m; ++k)
            if (fabs(a[k][i]) > fabs(a[piv][i]))
                piv = k;
        if (fabs(a[piv][i]) < 1e-300)
            return 1;
        for (j = 0; j <= m; ++j) {
            double t = a[i][j];
            a[i][j] = a[piv][j];
            a[piv][j] = t;
        }
        for (k = 0; k < m; ++k)
            if (k != i) {
                double f = a[k][i] / a[i][i];
                for (j = i; j <= m; ++j)
                    a[k][j] -= f * a[i][j];
            }
    }
    for (i = 0; i < m; ++i)
        c[i] = a[i][m] / a[i][i];
    return 0;
}

/* ---- Constant / polynomials ---- */
static const FitItParam constant_p[] = {{"c", "", "", 0.0, -INF, INF}};
POINTWISE(constant, p[0] + 0.0 * x)
static int32_t constant_guess(const double *x, const double *y, size_t n, double *p)
{
    return polyfit(x, y, n, 0, p);
}

static const FitItParam linear_p[] = {
    {"slope", "", "", 1.0, -INF, INF},
    {"intercept", "", "", 0.0, -INF, INF},
};
POINTWISE(linear, p[0] * x + p[1])
static int32_t linear_guess(const double *x, const double *y, size_t n, double *p)
{
    double c[2];
    if (polyfit(x, y, n, 1, c))
        return 1;
    p[0] = c[1];
    p[1] = c[0];
    return 0;
}

static const FitItParam quadratic_p[] = {
    {"c0", "", "", 0.0, -INF, INF},
    {"c1", "", "", 0.0, -INF, INF},
    {"c2", "", "", 0.0, -INF, INF},
};
POINTWISE(quadratic, p[0] + x * (p[1] + x * p[2]))
static int32_t quadratic_guess(const double *x, const double *y, size_t n, double *p)
{
    return polyfit(x, y, n, 2, p);
}

static const FitItParam cubic_p[] = {
    {"c0", "", "", 0.0, -INF, INF},
    {"c1", "", "", 0.0, -INF, INF},
    {"c2", "", "", 0.0, -INF, INF},
    {"c3", "", "", 0.0, -INF, INF},
};
POINTWISE(cubic, p[0] + x * (p[1] + x * (p[2] + x * p[3])))
static int32_t cubic_guess(const double *x, const double *y, size_t n, double *p)
{
    return polyfit(x, y, n, 3, p);
}

/* ---- Exponential / power law: linear fits in log space ---- */
static int log_fit(const double *x, const double *y, size_t n, int log_x, double *slope, double *icpt)
{
    double sx = 0, sy = 0, sxx = 0, sxy = 0, m = 0, d;
    size_t i;
    for (i = 0; i < n; ++i) {
        double u = log_x ? log(x[i]) : x[i], v = log(y[i]);
        if (!isfinite(u) || !isfinite(v))
            continue;
        sx += u;
        sy += v;
        sxx += u * u;
        sxy += u * v;
        m += 1;
    }
    d = m * sxx - sx * sx;
    if (m < 2 || d == 0.0)
        return 1;
    *slope = (m * sxy - sx * sy) / d;
    *icpt = (sy - *slope * sx) / m;
    return 0;
}

static const FitItParam exponential_p[] = {
    {"amplitude", "", "", 1.0, -INF, INF},
    {"decay", "", "", 1.0, -INF, INF},
};
POINTWISE(exponential, p[0] * exp(-x / p[1]))
static int32_t exponential_guess(const double *x, const double *y, size_t n, double *p)
{
    double s, c;
    if (log_fit(x, y, n, 0, &s, &c) || s == 0.0)
        return 1;
    p[0] = exp(c);
    p[1] = -1.0 / s;
    return 0;
}

static const FitItParam powerlaw_p[] = {
    {"amplitude", "", "", 1.0, -INF, INF},
    {"exponent", "", "", 1.0, -INF, INF},
};
POINTWISE(powerlaw, p[0] * pow(x, p[1]))
static int32_t powerlaw_guess(const double *x, const double *y, size_t n, double *p)
{
    double s, c;
    if (log_fit(x, y, n, 1, &s, &c))
        return 1;
    p[0] = exp(c);
    p[1] = s;
    return 0;
}

/* ---- Steps ---- */
static const FitItParam step_p[] = {
    {"amplitude", "", "step height", 1.0, -INF, INF},
    {"center", "", "", 0.0, -INF, INF},
    {"sigma", "", "step width", 1.0, 0.0, INF},
};
POINTWISE(step_erf, p[0] * 0.5 * (1.0 + erf((x - p[1]) / (p[2] * sqrt(2.0)))))
POINTWISE(step_logistic, p[0] / (1.0 + exp(-(x - p[1]) / p[2])))
static int32_t step_guess(const double *x, const double *y, size_t n, double *p)
{
    size_t i, lo = 0, hi = 0;
    for (i = 1; i < n; ++i) {
        if (x[i] < x[lo])
            lo = i;
        if (x[i] > x[hi])
            hi = i;
    }
    if (n < 2 || x[hi] == x[lo])
        return 1;
    p[0] = y[hi] - y[lo];
    p[1] = 0.5 * (x[lo] + x[hi]);
    p[2] = (x[hi] - x[lo]) / 10.0;
    return 0;
}

#define COUNT(a) ((uint32_t)(sizeof(a) / sizeof((a)[0])))

static const FitItModel models[] = {
    {FIT_IT_ABI_VERSION, COUNT(constant_p), "Constant", "Background", "c", constant_p, constant, constant_guess},
    {FIT_IT_ABI_VERSION, COUNT(linear_p), "Linear", "Background", "slope * x + intercept", linear_p, linear, linear_guess},
    {FIT_IT_ABI_VERSION, COUNT(quadratic_p), "Quadratic", "Background", "c0 + c1 x + c2 x^2", quadratic_p, quadratic, quadratic_guess},
    {FIT_IT_ABI_VERSION, COUNT(cubic_p), "Cubic", "Background", "c0 + c1 x + c2 x^2 + c3 x^3", cubic_p, cubic, cubic_guess},
    {FIT_IT_ABI_VERSION, COUNT(exponential_p), "Exponential", "Decay", "amplitude * exp(-x / decay)", exponential_p, exponential, exponential_guess},
    {FIT_IT_ABI_VERSION, COUNT(powerlaw_p), "PowerLaw", "Background", "amplitude * x^exponent", powerlaw_p, powerlaw, powerlaw_guess},
    {FIT_IT_ABI_VERSION, COUNT(step_p), "StepErf", "Step", "amplitude/2 * (1 + erf((x-center) / (sigma sqrt 2)))", step_p, step_erf, step_guess},
    {FIT_IT_ABI_VERSION, COUNT(step_p), "StepLogistic", "Step", "amplitude / (1 + exp(-(x-center)/sigma))", step_p, step_logistic, step_guess},
};

FIT_IT_EXPORT const FitItModel *fit_it_models(uint32_t *count)
{
    *count = COUNT(models);
    return models;
}
