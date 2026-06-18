#!/usr/bin/env python3
"""Fit the Obric closed form to *real* SolFi V2 fills, with no LiteSVM/Solana toolchain needed.

Ground truth comes from historical on-chain swaps (their token-balance deltas), so this runs
anywhere Python + an RPC are available. It is the verifiable counterpart to `propquote-sim`
(which runs the venue `.so` in LiteSVM) — same goal, different ground-truth source.

Usage:
    SOLANA_RPC_URL=<mainnet rpc> python tools/solfi_fit.py
    python tools/solfi_fit.py <mainnet rpc url>

Method:
  1. Pull recent transactions touching the SolFi V2 SOL/USDC market.
  2. For each clean WSOL->USDC fill, read amount_in / amount_out / pre-trade reserves from the
     vault token-balance deltas (no instruction decoding needed).
  3. Infer the oracle scale from the smallest fill, then fit (big_k, fee) of the Obric form.
  4. Report prediction error in basis points.

Caveats (honest): arb flow clusters around one size, so curvature (big_k) is loosely identified,
and spread cannot be separated from the oracle without an external price feed (it folds into the
inferred scale). For *predicting* a venue's output — which is what arb needs — this is fine.
"""
import json
import math
import os
import sys
import urllib.request

MARKET = "65ZHSArs5XxPseKQbB1B4r16vDxMWnCxHMzogDAqiDUc"
BASE_VAULT = "CRo8DBwrmd97DJfAnvCv96tZPL5Mktf2NZy2ZnhDer1A"   # WSOL, 9 decimals
QUOTE_VAULT = "GhFfLFSprPpfoRaWakPMmJTMJBHuz6C694jYwxy2dAic"  # USDC, 6 decimals
SCALE = 1_000_000  # mult_y; mult_x is the price in these units


def rpc(url, method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=40)).get("result")


def collect_fills(url, limit=150):
    sigs = rpc(url, "getSignaturesForAddress", [MARKET, {"limit": limit}]) or []
    samples, times = [], []
    for s in sigs:
        if s.get("err"):
            continue
        tx = rpc(url, "getTransaction",
                 [s["signature"], {"encoding": "jsonParsed", "maxSupportedTransactionVersion": 0}])
        if not tx:
            continue
        meta = tx.get("meta") or {}
        msg = tx["transaction"]["message"]
        loaded = meta.get("loadedAddresses") or {}
        combined = ([k["pubkey"] for k in msg.get("accountKeys", [])]
                    + loaded.get("writable", []) + loaded.get("readonly", []))

        def balances(bals):
            out = {}
            for b in bals or []:
                i = b["accountIndex"]
                if i < len(combined):
                    out[combined[i]] = int(b["uiTokenAmount"]["amount"])
            return out

        pre, post = balances(meta.get("preTokenBalances")), balances(meta.get("postTokenBalances"))
        if BASE_VAULT not in pre or QUOTE_VAULT not in pre:
            continue
        bd = post.get(BASE_VAULT, pre[BASE_VAULT]) - pre[BASE_VAULT]
        qd = post.get(QUOTE_VAULT, pre[QUOTE_VAULT]) - pre[QUOTE_VAULT]
        if bd > 0 and qd < 0:  # clean WSOL -> USDC
            samples.append({"cx": pre[BASE_VAULT], "cy": pre[QUOTE_VAULT], "ain": bd, "aout": -qd})
            if tx.get("blockTime"):
                times.append(tx["blockTime"])
    return samples, times


def predict(ain, cy, mult_x, big_k, fee):
    # target_x == current_x, so current_x_k == target_x_k (quotes centered on the oracle).
    txk = math.isqrt(big_k * SCALE // mult_x)
    if txk == 0:
        return None
    cyk = big_k // txk
    nxk = txk + ain
    ob = cyk - big_k // nxk
    if ob <= 0 or ob >= cy:
        return None
    return ob - (ob * fee // 1_000_000)


def fit(samples, mult_x):
    def terr(big_k, fee):
        acc = 0
        for s in samples:
            o = predict(s["ain"], s["cy"], mult_x, big_k, fee)
            acc += (1 << 80) if o is None else abs(o - s["aout"])
        return acc

    def best_fee(big_k):
        lo, hi = 0, 200_000
        while hi - lo > 2:
            t = (hi - lo) // 3
            m1, m2 = lo + t, hi - t
            hi, lo = (m2, lo) if terr(big_k, m1) <= terr(big_k, m2) else (hi, m1)
        return min(range(lo, hi + 1), key=lambda f: terr(big_k, f))

    def err(big_k):
        return terr(big_k, best_fee(big_k))

    big_k = min((1 << i for i in range(20, 121)), key=err)
    step = max(1, big_k // 2)
    while True:
        moved = True
        while moved:
            moved = False
            for c in (big_k - step, big_k + step):
                if c > 0 and err(c) < err(big_k):
                    big_k, moved = c, True
        if step == 1:
            break
        step = max(1, step // 2)
    return big_k, best_fee(big_k)


def main():
    url = sys.argv[1] if len(sys.argv) > 1 else os.environ.get("SOLANA_RPC_URL")
    if not url:
        sys.exit("set SOLANA_RPC_URL or pass the RPC url as an argument")

    samples, times = collect_fills(url)
    print(f"{len(samples)} clean WSOL->USDC fills" + (f" over {max(times) - min(times)}s" if times else ""))
    if len(samples) < 5:
        sys.exit("not enough samples to fit")

    samples.sort(key=lambda s: s["ain"])
    sm = samples[0]
    mult_x = round(sm["aout"] / sm["ain"] * SCALE)
    print(f"oracle ~{sm['aout'] / sm['ain'] * 1000:.2f} USDC/SOL   "
          f"sizes {samples[0]['ain'] / 1e9:.3f}..{samples[-1]['ain'] / 1e9:.3f} SOL")

    big_k, fee = fit(samples, mult_x)
    bps = sorted(abs(predict(s["ain"], s["cy"], mult_x, big_k, fee) - s["aout"]) / s["aout"] * 1e4
                 for s in samples if predict(s["ain"], s["cy"], mult_x, big_k, fee))
    print(f"\nbig_k={big_k}  fee_millionth={fee}")
    print(f"prediction error: median {bps[len(bps) // 2]:.2f} bps  "
          f"p90 {bps[int(len(bps) * 0.9)]:.2f} bps  max {bps[-1]:.2f} bps")


if __name__ == "__main__":
    main()
