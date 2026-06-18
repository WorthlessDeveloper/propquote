#!/usr/bin/env python3
"""Fit the Obric closed form to real on-chain fills for any prop AMM SOL/USDC pool.

Generalises tools/solfi_fit.py: ground truth is each pool's vault token-balance deltas, so it works
for any venue once you know its two reserve vaults — no instruction decoding, no LiteSVM, no Solana
toolchain. Verifiable anywhere Python + an RPC are available.

Usage:
    SOLANA_RPC_URL=<rpc> python tools/fit_venue.py [venue ...]
    python tools/fit_venue.py <rpc> [venue ...]

Pool addresses are from LimeChain/magnus cfg/payloads/pmms.json (edit if stale). BisonFi is omitted
(no public pool address yet). Caveats are the same as solfi_fit.py: arb flow clusters around one
trade size (curvature loosely identified) and spread folds into the inferred oracle.
"""
import json
import math
import os
import sys
import urllib.request

SCALE = 1_000_000

# venue -> (market/pool used for getSignaturesForAddress, base vault [WSOL], quote vault [USDC])
VENUES = {
    "solfi-v2": ("65ZHSArs5XxPseKQbB1B4r16vDxMWnCxHMzogDAqiDUc",
                 "CRo8DBwrmd97DJfAnvCv96tZPL5Mktf2NZy2ZnhDer1A",
                 "GhFfLFSprPpfoRaWakPMmJTMJBHuz6C694jYwxy2dAic"),
    "zerofi": ("2h9hhu3gxY9kCdXEwdTHV8yPAMYVoHgKopRyG1HbDwfi",
               "ERP5RTV6cWmoGrv7r9W2V5pbgDFSepc4j97qNnx1Jris",
               "7wYJVD8iXmMQjND1fwi1hPr68QwruVVtirbotyJZXaVH"),
    "tessera": ("FLckHLGMJy5gEoXWwcE68Nprde1D4araK4TGLw4pQq2n",
                "5pVN5XZB8cYBjNLFrsBCPWkCQBan5K5Mq2dWGzwPgGJV",
                "9t4P5wMwfFkyn92Z7hf463qYKEZf8ERVZsGBEPNp8uJx"),
    "humidifi": ("FksffEqnBRixYGR791Qw2MgdU7zNCpHVFYBL4Fa4qVuH",
                 "C3FzbX9n1YD2dow2dCmEv5uNyyf22Gb3TLAEqGBhw5fY",
                 "3RWFAQBRkNGq7CMGcTLK3kXDgFTe9jgMeFYqk8nHwcWh"),
    "goonfi": ("4uWuh9fC7rrZKrN8ZdJf69MN1e2S7FPpMqcsyY1aof6K",
               "pKiUC9hDXv52xqU1p3BKypV9AQjAMgfZUGRnoBsdkKm",
               "Gsy5Zr7Vxn5KckAbduPHHGR1qzPJ4w3GSYmcinWAkhrC"),
}


def rpc(url, method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=40)).get("result")


def collect(url, market, base_vault, quote_vault, limit=120, want=24):
    sigs = rpc(url, "getSignaturesForAddress", [market, {"limit": limit}]) or []
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

        def bals(b):
            d = {}
            for x in b or []:
                i = x["accountIndex"]
                if i < len(combined):
                    d[combined[i]] = int(x["uiTokenAmount"]["amount"])
            return d

        pre, post = bals(meta.get("preTokenBalances")), bals(meta.get("postTokenBalances"))
        if base_vault not in pre or quote_vault not in pre:
            continue
        bd = post.get(base_vault, pre[base_vault]) - pre[base_vault]
        qd = post.get(quote_vault, pre[quote_vault]) - pre[quote_vault]
        if bd > 0 and qd < 0:  # clean WSOL -> USDC
            samples.append({"cy": pre[quote_vault], "ain": bd, "aout": -qd})
            if tx.get("blockTime"):
                times.append(tx["blockTime"])
        if len(samples) >= want:
            break
    return samples, times


def predict(ain, cy, mult_x, big_k, fee):
    txk = math.isqrt(big_k * SCALE // mult_x)
    if txk == 0:
        return None
    ob = big_k // txk - big_k // (txk + ain)
    if ob <= 0 or ob >= cy:
        return None
    return ob - (ob * fee // 1_000_000)


def fit(samples, mult_x):
    def terr(big_k, fee):
        return sum((1 << 80) if (o := predict(s["ain"], s["cy"], mult_x, big_k, fee)) is None
                   else abs(o - s["aout"]) for s in samples)

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


def run(url, venue):
    market, base_vault, quote_vault = VENUES[venue]
    try:
        samples, times = collect(url, market, base_vault, quote_vault)
    except Exception as e:  # noqa: BLE001 - report and continue
        print(f"{venue:10} ERROR: {e}")
        return
    span = f"{max(times) - min(times)}s" if times else "?"
    if len(samples) < 5:
        print(f"{venue:10} only {len(samples)} clean WSOL->USDC fills (low volume / stale pool)")
        return
    samples.sort(key=lambda s: s["ain"])
    sm = samples[0]
    mult_x = round(sm["aout"] / sm["ain"] * SCALE)
    big_k, fee = fit(samples, mult_x)
    bps = sorted(abs(predict(s["ain"], s["cy"], mult_x, big_k, fee) - s["aout"]) / s["aout"] * 1e4
                 for s in samples if predict(s["ain"], s["cy"], mult_x, big_k, fee))
    print(f"{venue:10} {len(samples):>3} fills/{span:>5}  "
          f"~{sm['aout'] / sm['ain'] * 1000:6.2f} USDC/SOL  "
          f"sizes {samples[0]['ain'] / 1e9:6.3f}-{samples[-1]['ain'] / 1e9:<7.3f} SOL  "
          f"err: med {bps[len(bps) // 2]:5.2f} / p90 {bps[int(len(bps) * 0.9)]:5.2f} / max {bps[-1]:5.2f} bps")


def main():
    args = sys.argv[1:]
    url = os.environ.get("SOLANA_RPC_URL")
    if args and "://" in args[0]:
        url = args.pop(0)
    if not url:
        sys.exit("set SOLANA_RPC_URL or pass the RPC url first")
    venues = args or list(VENUES)
    print(f"{'venue':10} {'fills':>9}  {'oracle':>15}  {'size range':>22}  prediction error")
    print("-" * 100)
    for v in venues:
        if v in VENUES:
            run(url, v)
        else:
            print(f"{v:10} unknown venue (have: {', '.join(VENUES)})")


if __name__ == "__main__":
    main()
