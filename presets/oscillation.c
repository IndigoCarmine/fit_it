/*
 * fit_it preset: periodic signals.
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

static const FitItParam sine_p[] = {
    {"amplitude", "", "", 1.0, -INF, INF},
    {"frequency", "", "angular frequency", 1.0, 0.0, INF},
    {"shift", "rad", "phase", 0.0, -INF, INF},
};
POINTWISE(sine, p[0] * sin(p[1] * x + p[2]))

static const FitItParam damped_p[] = {
    {"amplitude", "", "", 1.0, -INF, INF},
    {"frequency", "", "cycles per unit x", 1.0, 0.0, INF},
    {"phase", "rad", "", 0.0, -INF, INF},
    {"decay", "", "", 1.0, 0.0, INF},
};
POINTWISE(damped_sine, p[0] * exp(-x / p[3]) * sin(2.0 * PI * p[1] * x + p[2]))

#define COUNT(a) ((uint32_t)(sizeof(a) / sizeof((a)[0])))

static const FitItModel models[] = {
    {FIT_IT_ABI_VERSION, COUNT(sine_p), "Sine", "Oscillation", "amplitude * sin(frequency x + shift)", sine_p, sine, NULL},
    {FIT_IT_ABI_VERSION, COUNT(damped_p), "DampedSine", "Oscillation",
     "amplitude * exp(-x/decay) * sin(2 pi frequency x + phase)", damped_p, damped_sine, NULL},
};

FIT_IT_EXPORT const FitItModel *fit_it_models(uint32_t *count)
{
    *count = COUNT(models);
    return models;
}
