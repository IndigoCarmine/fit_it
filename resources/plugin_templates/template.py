r"""
fit_it Python model: MyModel

The format follows SasView plugin models. Save and press "Reload models".

Two styles are supported:
  * Iq(q, *params)  -- SasView style; `scale` and `background` are added for you:
                       I(q) = scale * Iq(q, ...) / form_volume(...) + background
                       (form_volume is optional and takes the "volume" params)
  * f(x, *params)   -- a plain function y = f(x, ...), used as is.

The function receives a numpy array and should return an array of the same
length (scalar-only functions are vectorised automatically, but slower).
"""
import numpy as np

name = "MyModel"
title = "Short title"
description = "I(q) = A / (1 + (q xi)^m)"
category = "Custom"

# SasView rows: [name, units, default, [lower, upper], type, description]
# type "volume" marks parameters passed to form_volume.
# Short forms also work: ("name", default) or ("name", default, lower, upper)
parameters = [
    ["A", "", 1.0, [0, np.inf], "", "amplitude"],
    ["xi", "Ang", 20.0, [0, np.inf], "", "correlation length"],
    ["m", "", 2.0, [0, 6], "", "exponent"],
]


def Iq(q, A=1.0, xi=20.0, m=2.0):
    return A / (1.0 + (q * xi) ** m)


Iq.vectorized = True


# Optional: initial values for the parameters declared above.
# def guess(x, y):
#     return [y.max(), 1.0 / x[np.argmax(y < y.max() / 2)], 2.0]
