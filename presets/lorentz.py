r"""
fit_it preset: Ornstein-Zernike (Lorentz) scattering, in SasView model format.

Any SasView-style plugin model works the same way: `name`, `description`,
`category`, `parameters` and a vectorised `Iq(q, *params)`. As in SasView,
`scale` and `background` are added automatically for `Iq` models:

    I(q) = scale * Iq(q, ...) + background

Needs Python with numpy (configure the interpreter under Plugins).
"""
import numpy as np

name = "OrnsteinZernike"
title = "Ornstein-Zernike (Lorentz) model"
description = "I(q) = scale / (1 + (q cor_length)^2) + background"
category = "SAS"

#   [ "name", "units", default, [lower, upper], "type", "description"],
parameters = [
    ["cor_length", "Ang", 50.0, [0, np.inf], "", "Screening length"],
]


def Iq(q, cor_length=50.0):
    return 1.0 / (1.0 + (q * cor_length) ** 2)


Iq.vectorized = True
