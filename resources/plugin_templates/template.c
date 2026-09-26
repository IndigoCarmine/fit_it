/*
 * fit_it C model: MyModel
 *
 * Save changes and press "Reload models" in fit_it -- the app compiles this file
 * with the C compiler it finds (CC, cl, clang, gcc) and loads it.
 * fit_it_plugin.h sits in the same folder and documents the ABI.
 */
#include "fit_it_plugin.h"

#include <math.h>

/* name, unit, description, default, min, max (use HUGE_VAL for infinity) */
static const FitItParam params[] = {
    {"amplitude", "", "", 1.0, -HUGE_VAL, HUGE_VAL},
    {"decay", "", "decay constant", 1.0, 0.0, HUGE_VAL},
    {"offset", "", "", 0.0, -HUGE_VAL, HUGE_VAL},
};

/* out[i] = f(x[i]; p). May run on several threads at once: no global state. */
static int32_t eval(const double *x, size_t n, const double *p, double *out)
{
    for (size_t i = 0; i < n; ++i)
        out[i] = p[0] * exp(-x[i] / p[1]) + p[2];
    return 0; /* non-zero reports an error */
}

/* Optional initial guess from data; return non-zero to skip. */
static int32_t guess(const double *x, const double *y, size_t n, double *p)
{
    double ymin = y[0], ymax = y[0], xmin = x[0], xmax = x[0];
    for (size_t i = 1; i < n; ++i) {
        ymin = fmin(ymin, y[i]);
        ymax = fmax(ymax, y[i]);
        xmin = fmin(xmin, x[i]);
        xmax = fmax(xmax, x[i]);
    }
    p[0] = ymax - ymin;
    p[1] = (xmax - xmin) / 3.0;
    p[2] = ymin;
    return 0;
}

static const FitItModel models[] = {
    {FIT_IT_ABI_VERSION, sizeof(params) / sizeof(params[0]), "MyModel", "Custom",
     "amplitude * exp(-x / decay) + offset", params, eval, guess},
};

FIT_IT_EXPORT const FitItModel *fit_it_models(uint32_t *count)
{
    *count = sizeof(models) / sizeof(models[0]);
    return models;
}
