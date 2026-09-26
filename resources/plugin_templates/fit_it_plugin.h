/*
 * fit_it plugin ABI (version 2; version-1 libraries still load)
 *
 * A plugin is a shared library (.dll / .so / .dylib) exporting one function,
 * `fit_it_models`, that returns an array of model descriptions. Drop a .c file
 * next to this header into a fit_it model folder and the app compiles it for
 * you; a prebuilt library works too.
 *
 * Rules:
 *  - Everything returned must stay valid while the library is loaded
 *    (use static storage).
 *  - `eval` may be called from several threads at once: keep it free of
 *    global mutable state.
 *  - Return 0 on success, anything else to report a failure.
 */
#ifndef FIT_IT_PLUGIN_H
#define FIT_IT_PLUGIN_H

#include <stddef.h>
#include <stdint.h>

#define FIT_IT_ABI_VERSION 2

/* FitItParam.flags */
#define FIT_IT_PARAM_FIXED 1u /* not fitted by default (e.g. a known concentration) */

#ifdef _WIN32
#define FIT_IT_EXPORT __declspec(dllexport)
#else
#define FIT_IT_EXPORT __attribute__((visibility("default")))
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef struct FitItParam {
    const char *name;        /* identifier: letters, digits, '_' */
    const char *unit;        /* may be "" */
    const char *description; /* may be "" */
    double default_value;
    double min;              /* -INFINITY for no bound */
    double max;              /* INFINITY for no bound */
    /* The two fields below may be left out of an initializer (they become 0/NULL). */
    uint32_t flags;          /* FIT_IT_PARAM_FIXED, ... */
    const char *expr;        /* default constraint, e.g. "conc_M"; used only when the
                                dataset defines every name in it. NULL = none */
} FitItParam;

/* out[i] = f(x[i]; p) for i in 0..n */
typedef int32_t (*FitItEvalFn)(const double *x, size_t n, const double *p, double *out);

/* Optional: write initial guesses for all parameters into p_out. */
typedef int32_t (*FitItGuessFn)(const double *x, const double *y, size_t n, double *p_out);

typedef struct FitItModel {
    uint32_t abi_version;     /* FIT_IT_ABI_VERSION */
    uint32_t n_params;
    const char *name;         /* unique model name shown in the app */
    const char *category;     /* e.g. "Peak", "Background", "SAS" */
    const char *description;
    const FitItParam *params; /* n_params entries */
    FitItEvalFn eval;
    FitItGuessFn guess;       /* may be NULL */
} FitItModel;

/* Implement this: set *count and return a pointer to *count models. */
FIT_IT_EXPORT const FitItModel *fit_it_models(uint32_t *count);

#ifdef __cplusplus
}
#endif

#endif /* FIT_IT_PLUGIN_H */
