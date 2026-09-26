//! Global search for χ² landscapes with many local minima (multi-peak spectra,
//! oscillatory models, …). Each method only produces a start point; [`super::fit`]
//! always finishes with a Levenberg–Marquardt run from it, so the reported
//! uncertainties come from the same code as a plain LM fit.
//!
//! The methods work on the free parameters in external units inside a finite
//! search box ([`super::search_box`]): the parameter bounds where they are finite,
//! a range around the current value where they are not. Constraints are applied by
//! [`Problem::values_with`](super::Problem::values_with) on every evaluation.
//!
//! * **Multi-start LM** — Latin-hypercube sample the box, run LM from the current
//!   values and from the best samples. Cheap and very effective for 2–10 parameters.
//! * **Differential evolution** — L-SHADE (Tanabe & Fukunaga 2014): success-history
//!   adaptation of F/CR, `current-to-pbest/1` mutation with an archive, and linear
//!   population size reduction. Derivative-free and robust on rugged landscapes.
//! * **Basin hopping** — Wales & Doye (1997), as in SciPy: random jumps between LM
//!   minima with Metropolis acceptance and an adaptive step size.

use super::{
    Evaluator, FitAlgorithm, FitOptions, Progress, cancelled, levenberg_marquardt, report_progress,
    search_box, to_external,
};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

pub(super) struct GlobalResult {
    /// Best free parameter values found (external units).
    pub free: Vec<f64>,
    pub cost: f64,
    /// Generations / LM iterations spent, added to the reported iteration count.
    pub iters: usize,
    /// One-line summary for the fit message.
    pub note: String,
}

/// Small deterministic PRNG (xoshiro256**, seeded through SplitMix64).
pub(super) struct Rng([u64; 4]);

impl Rng {
    pub fn new(seed: u64) -> Self {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        Self([next(), next(), next(), next()])
    }

    pub fn next_u64(&mut self) -> u64 {
        let s = &mut self.0;
        let out = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }

    /// Uniform in [0, 1).
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
    }

    /// Uniform integer in 0..n (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        ((self.uniform() * n as f64) as usize).min(n - 1)
    }

    pub fn normal(&mut self, mean: f64, sd: f64) -> f64 {
        let u1 = 1.0 - self.uniform();
        let u2 = self.uniform();
        mean + sd * (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
    }

    pub fn cauchy(&mut self, loc: f64, scale: f64) -> f64 {
        loc + scale * (std::f64::consts::PI * (self.uniform() - 0.5)).tan()
    }

    fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            v.swap(i, self.below(i + 1));
        }
    }
}

/// `m` Latin-hypercube points in the box.
fn latin_hypercube(m: usize, bx: &[(f64, f64)], rng: &mut Rng) -> Vec<Vec<f64>> {
    let mut pts = vec![vec![0.0; bx.len()]; m];
    let mut perm: Vec<usize> = (0..m).collect();
    for (j, &(lo, hi)) in bx.iter().enumerate() {
        rng.shuffle(&mut perm);
        for (p, &k) in pts.iter_mut().zip(&perm) {
            p[j] = lo + (k as f64 + rng.uniform()) / m as f64 * (hi - lo);
        }
    }
    pts
}

struct Ctx<'a, 'b> {
    ev: &'a mut Evaluator<'b>,
    opts: &'a FitOptions,
    bx: Vec<(f64, f64)>,
    rng: Rng,
    /// `ev.nfev` at which the global stage must stop.
    nfev_end: usize,
    progress: Option<&'a Mutex<Progress>>,
    cancel: Option<&'a AtomicBool>,
}

impl Ctx<'_, '_> {
    fn out_of_budget(&self) -> bool {
        self.ev.nfev >= self.nfev_end || cancelled(self.cancel)
    }

    /// Local LM from external point `x`; returns the minimum (external) and χ².
    fn local(&mut self, x: &[f64]) -> Option<(Vec<f64>, f64, usize)> {
        let u = self.ev.internal(x);
        let r = self.ev.residuals(&u)?;
        // Exploratory runs do not need the final run's tight tolerances.
        let opts = FitOptions {
            ftol: self.opts.ftol.max(1e-8),
            xtol: self.opts.xtol.max(1e-8),
            ..*self.opts
        };
        let limit = (self.ev.nfev + self.opts.max_nfev).min(self.nfev_end.max(self.ev.nfev + 1));
        let run = levenberg_marquardt(self.ev, u, r, &opts, limit, 0, None, self.cancel);
        let free = run
            .u
            .iter()
            .zip(&self.ev.problem.free)
            .map(|(&u, &i)| to_external(u, self.ev.problem.bounds[i]))
            .collect();
        Some((free, run.cost, run.niter))
    }
}

pub(super) fn search(
    ev: &mut Evaluator,
    opts: &FitOptions,
    init_free: &[f64],
    progress: Option<&Mutex<Progress>>,
    cancel: Option<&AtomicBool>,
) -> GlobalResult {
    let bx: Vec<(f64, f64)> = init_free
        .iter()
        .zip(&ev.problem.free)
        .map(|(&v, &i)| search_box(v, ev.problem.bounds[i], opts.search_width))
        .collect();
    let mut cx = Ctx {
        nfev_end: ev.nfev + opts.global_max_nfev.max(1),
        ev,
        opts,
        bx,
        rng: Rng::new(opts.seed),
        progress,
        cancel,
    };
    match opts.algorithm {
        FitAlgorithm::MultiStart => multi_start(&mut cx, init_free),
        FitAlgorithm::DifferentialEvolution => differential_evolution(&mut cx, init_free),
        FitAlgorithm::BasinHopping => basin_hopping(&mut cx, init_free),
        FitAlgorithm::LevenbergMarquardt => GlobalResult {
            free: init_free.to_vec(),
            cost: cx.ev.cost_ext(init_free),
            iters: 0,
            note: String::new(),
        },
    }
}

// ---------------------------------------------------------------------------
// Multi-start LM

fn multi_start(cx: &mut Ctx, init: &[f64]) -> GlobalResult {
    let k = cx.opts.global_starts.max(1);
    // Screen a larger sample and start LM from the best points; a quarter of the
    // budget at most goes to screening.
    let nsample = (10 * k).max(50).min(cx.opts.global_max_nfev / 4).max(k);
    let mut samples: Vec<(f64, Vec<f64>)> = Vec::with_capacity(nsample);
    for x in latin_hypercube(nsample, &cx.bx, &mut cx.rng) {
        if cx.out_of_budget() {
            break;
        }
        samples.push((cx.ev.cost_ext(&x), x));
    }
    samples.retain(|(c, _)| c.is_finite());
    samples.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut starts = vec![init.to_vec()];
    starts.extend(samples.into_iter().take(k - 1).map(|(_, x)| x));

    let (mut best, mut best_cost) = (init.to_vec(), f64::INFINITY);
    let (mut runs, mut iters) = (0, 0);
    for x in &starts {
        if cx.out_of_budget() && runs > 0 {
            break;
        }
        let Some((xm, c, it)) = cx.local(x) else {
            continue;
        };
        runs += 1;
        iters += it;
        if c < best_cost {
            best_cost = c;
            best = xm;
        }
        report_progress(cx.progress, runs, cx.ev.nfev, best_cost);
    }
    GlobalResult {
        note: format!("multi-start: {runs} LM runs, best χ² = {best_cost:.6e}"),
        free: best,
        cost: best_cost,
        iters,
    }
}

// ---------------------------------------------------------------------------
// Basin hopping

fn basin_hopping(cx: &mut Ctx, init: &[f64]) -> GlobalResult {
    let hops = cx.opts.global_starts.max(1);
    let n = init.len();
    let mut iters = 0;
    let mut cur = cx.local(init);
    if cur.is_none() {
        // The current values do not evaluate; start from the best of a small sample.
        let best = latin_hypercube(20, &cx.bx, &mut cx.rng)
            .into_iter()
            .map(|x| (cx.ev.cost_ext(&x), x))
            .filter(|(c, _)| c.is_finite())
            .min_by(|a, b| a.0.total_cmp(&b.0));
        cur = best.and_then(|(_, x)| cx.local(&x));
    }
    let Some((mut x, mut cost, it)) = cur else {
        return GlobalResult {
            free: init.to_vec(),
            cost: f64::INFINITY,
            iters,
            note: "basin hopping: no valid start".into(),
        };
    };
    iters += it;
    let (mut best, mut best_cost) = (x.clone(), cost);
    // Step as a fraction of each box width; adapted towards 50 % acceptance.
    let mut step = 0.25f64;
    // Metropolis temperature relative to the current χ².
    let temperature = 0.2;
    let (mut accepted, mut done) = (0usize, 0usize);
    for hop in 1..=hops {
        if cx.out_of_budget() {
            break;
        }
        let trial: Vec<f64> = (0..n)
            .map(|j| {
                let (lo, hi) = cx.bx[j];
                let w = hi - lo;
                let mut v = x[j] + step * w * (2.0 * cx.rng.uniform() - 1.0);
                // Reflect back into the box.
                if w > 0.0 {
                    for _ in 0..4 {
                        if v < lo {
                            v = 2.0 * lo - v;
                        } else if v > hi {
                            v = 2.0 * hi - v;
                        }
                    }
                    v = v.clamp(lo, hi);
                } else {
                    v = lo;
                }
                v
            })
            .collect();
        done += 1;
        if let Some((xm, c, it)) = cx.local(&trial) {
            iters += it;
            let accept = c < cost
                || cx.rng.uniform() < (-(c - cost) / (temperature * cost.max(1e-300))).exp();
            if accept {
                accepted += 1;
                x = xm;
                cost = c;
                if cost < best_cost {
                    best_cost = cost;
                    best = x.clone();
                }
            }
        }
        if hop % 10 == 0 {
            let rate = accepted as f64 / done as f64;
            step = if rate > 0.5 { step / 0.9 } else { step * 0.9 }.clamp(0.01, 1.0);
            accepted = 0;
            done = 0;
        }
        report_progress(cx.progress, hop, cx.ev.nfev, best_cost);
    }
    GlobalResult {
        note: format!("basin hopping: best χ² = {best_cost:.6e}"),
        free: best,
        cost: best_cost,
        iters,
    }
}

// ---------------------------------------------------------------------------
// L-SHADE differential evolution

fn differential_evolution(cx: &mut Ctx, init: &[f64]) -> GlobalResult {
    const H: usize = 6;
    const P_BEST: f64 = 0.11;
    const NP_MIN: usize = 4;
    let d = init.len();
    let np0 = if cx.opts.population > 0 {
        cx.opts.population.max(NP_MIN + 1)
    } else {
        (18 * d).clamp(30, 200)
    };
    let max_evals = cx.opts.global_max_nfev.max(np0);
    let nfev0 = cx.ev.nfev;

    let mut pop = latin_hypercube(np0, &cx.bx, &mut cx.rng);
    pop[0] = init.to_vec();
    let mut cost: Vec<f64> = Vec::with_capacity(np0);
    for x in &pop {
        if cancelled(cx.cancel) {
            break;
        }
        cost.push(cx.ev.cost_ext(x));
    }
    if cost.len() < NP_MIN {
        // Cancelled before the population was complete.
        let c = cost.first().copied().unwrap_or(f64::INFINITY);
        return GlobalResult {
            free: init.to_vec(),
            cost: if c.is_finite() {
                c
            } else {
                cx.ev.cost_ext(init)
            },
            iters: 0,
            note: "differential evolution: cancelled".into(),
        };
    }
    pop.truncate(cost.len());
    let mut archive: Vec<Vec<f64>> = Vec::new();
    let (mut m_f, mut m_cr) = ([0.5f64; H], [0.5f64; H]);
    let mut k = 0;
    let mut generation = 0;
    let mut converged = false;

    while !cx.out_of_budget() {
        generation += 1;
        let np = pop.len();
        let mut order: Vec<usize> = (0..np).collect();
        order.sort_by(|&a, &b| cost[a].total_cmp(&cost[b]));
        let n_pbest = ((P_BEST * np as f64).round() as usize).clamp(2, np);

        let mut trials = Vec::with_capacity(np);
        let mut params = Vec::with_capacity(np);
        for i in 0..np {
            let r = cx.rng.below(H);
            let cr = cx.rng.normal(m_cr[r], 0.1).clamp(0.0, 1.0);
            let f = loop {
                let f = cx.rng.cauchy(m_f[r], 0.1);
                if f > 0.0 {
                    break f.min(1.0);
                }
            };
            let pb = order[cx.rng.below(n_pbest)];
            let r1 = loop {
                let r1 = cx.rng.below(np);
                if r1 != i {
                    break r1;
                }
            };
            let r2 = loop {
                let r2 = cx.rng.below(np + archive.len());
                if r2 != i && r2 != r1 {
                    break r2;
                }
            };
            let x2 = if r2 < np { &pop[r2] } else { &archive[r2 - np] };
            let xi = &pop[i];
            let jrand = cx.rng.below(d);
            let trial: Vec<f64> = (0..d)
                .map(|j| {
                    if j != jrand && cx.rng.uniform() >= cr {
                        return xi[j];
                    }
                    let v = xi[j] + f * (pop[pb][j] - xi[j]) + f * (pop[r1][j] - x2[j]);
                    let (lo, hi) = cx.bx[j];
                    if v < lo {
                        (lo + xi[j]) / 2.0
                    } else if v > hi {
                        (hi + xi[j]) / 2.0
                    } else {
                        v
                    }
                })
                .collect();
            trials.push(trial);
            params.push((f, cr));
        }

        let (mut s_f, mut s_cr, mut s_w) = (Vec::new(), Vec::new(), Vec::new());
        for (i, trial) in trials.into_iter().enumerate() {
            if cx.out_of_budget() {
                break;
            }
            let c = cx.ev.cost_ext(&trial);
            if c <= cost[i] {
                if c < cost[i] {
                    let (f, cr) = params[i];
                    s_f.push(f);
                    s_cr.push(cr);
                    s_w.push(if cost[i].is_finite() {
                        cost[i] - c
                    } else {
                        1.0
                    });
                    archive.push(std::mem::replace(&mut pop[i], trial));
                } else {
                    pop[i] = trial;
                }
                cost[i] = c;
            }
        }

        // Success-history memory update (weighted Lehmer mean for F).
        let wsum: f64 = s_w.iter().sum();
        if !s_f.is_empty() && wsum > 0.0 && wsum.is_finite() {
            let lehmer = s_f.iter().zip(&s_w).map(|(f, w)| w * f * f).sum::<f64>()
                / s_f.iter().zip(&s_w).map(|(f, w)| w * f).sum::<f64>();
            let mean_cr = s_cr.iter().zip(&s_w).map(|(c, w)| w * c).sum::<f64>() / wsum;
            m_f[k] = lehmer;
            m_cr[k] = mean_cr;
            k = (k + 1) % H;
        }

        // Linear population size reduction: drop the worst.
        let used = (cx.ev.nfev - nfev0) as f64;
        let target = ((NP_MIN as f64 - np0 as f64) / max_evals as f64 * used + np0 as f64)
            .round()
            .max(NP_MIN as f64) as usize;
        if target < pop.len() {
            let mut idx: Vec<usize> = (0..pop.len()).collect();
            idx.sort_by(|&a, &b| cost[a].total_cmp(&cost[b]));
            idx.truncate(target);
            idx.sort_unstable();
            pop = idx.iter().map(|&i| pop[i].clone()).collect();
            cost = idx.iter().map(|&i| cost[i]).collect();
        }
        while archive.len() > pop.len() {
            let j = cx.rng.below(archive.len());
            archive.swap_remove(j);
        }

        let (lo, hi) = cost
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &c| {
                (a.min(c), b.max(c))
            });
        report_progress(cx.progress, generation, cx.ev.nfev, lo);
        if hi.is_finite() && hi - lo <= 1e-6 * lo.abs() + 1e-300 {
            converged = true;
            break;
        }
    }

    let (bi, &best_cost) = cost
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.total_cmp(b.1))
        .expect("population is never empty");
    GlobalResult {
        note: format!(
            "differential evolution: {generation} generations{}, best χ² = {best_cost:.6e}",
            if converged {
                " (population converged)"
            } else {
                ""
            }
        ),
        free: pop[bi].clone(),
        cost: best_cost,
        iters: generation,
    }
}
