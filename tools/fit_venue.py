#!/usr/bin/env python3
"""Fit the Obric closed form to real on-chain fills for any prop AMM SOL/USDC pool.

Robust version: instead of hardcoding (often stale) pool/vault addresses, it **discovers** each
venue's SOL/USDC vaults dynamically from the program's recent swaps — the pool's (WSOL, USDC) vault
pair is the token-account pair that recurs across the most transactions (user accounts vary; the
pool's don't). Ground truth is the vault token-balance deltas, so no instruction decoding, no
LiteSVM, no Solana toolchain — verifiable anywhere Python + an RPC are available.

Usage:
    SOLANA_RPC_URL=<rpc> python tools/fit_venue.py [venue ...]
    python tools/fit_venue.py <rpc> [venue ...]
"""
import json
import math
import os
import statistics
import sys
import urllib.request
from collections import Counter

WSOL = "So11111111111111111111111111111111111111112"
USDC = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
SCALE = 1_000_000
DUST_LAMPORTS = 10_000_000  # 0.01 SOL — below this the oracle inference is too noisy to trust
WINDOW_S = 30  # fit only fills within this many seconds so the oracle is ~constant (drift dominates otherwise)

# venue -> program id (vaults are discovered, not hardcoded)
VENUES = {
    "solfi-v2": "SV2EYYJyRz2YhfXwXnhNAevDEui5Q6yrfyo13WtupPF",
    "zerofi": "ZERor4xhbUycZ6gb9ntrhqscUcZmAbQDjEAtCf4hbZY",
    "tessera": "TessVdML9pBGgG9yGks7o4HewRaXVAMuoVj4x83GLQH",
    "humidifi": "9H6tua7jkLhdm3w8BvgpTn5LZNU7g4ZynDmCiNN3q6Rp",
    "goonfi": "goonERTdGsjnkZqWuVjs73BZ3Pb9qoCUdBUL17BnS5j",
    "bisonfi": "BiSoNHVpsVZW2F7rx2eQ59yQwKxzU5NvBcmKshCSUypi",
}

# Some venues sign far more oracle-update txs than swaps (HumidiFi pushes its price ~17x/sec), so
# getSignaturesForAddress(program) is mostly noise. For those, discover vaults from a known SOL/USDC
# pool/market account instead — its signatures are swaps, not oracle updates.
SIG_HINT = {
    "humidifi": "FksffEqnBRixYGR791Qw2MgdU7zNCpHVFYBL4Fa4qVuH",
}


def rpc(url, method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=40)).get("result")


def fetch_txs(url, program, limit=120):
    """Return a list of (entries, blockTime), entries = [(pubkey, mint, pre_amt, post_amt), ...]."""
    sigs = rpc(url, "getSignaturesForAddress", [program, {"limit": limit}]) or []
    out = []
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
        post = {b["accountIndex"]: int(b["uiTokenAmount"]["amount"]) for b in meta.get("postTokenBalances") or []}
        entries = []
        for b in meta.get("preTokenBalances") or []:
            i = b["accountIndex"]
            if i < len(combined):
                pre_amt = int(b["uiTokenAmount"]["amount"])
                entries.append((combined[i], b.get("mint"), pre_amt, post.get(i, pre_amt)))
        out.append((entries, tx.get("blockTime")))
    return out


def discover_vaults(txs):
    """The pool's (WSOL, USDC) vault pair is the changed-account pair recurring in the most txs."""
    pair = Counter()
    for entries, _ in txs:
        wsols = [pk for pk, m, pre, po in entries if m == WSOL and pre != po]
        usdcs = [pk for pk, m, pre, po in entries if m == USDC and pre != po]
        for w in set(wsols):
            for u in set(usdcs):
                pair[(w, u)] += 1
    if not pair:
        return None
    (base, quote), n = pair.most_common(1)[0]
    return base, quote, n


def extract_fills(txs, base, quote):
    samples = []
    for entries, bt in txs:
        d = {pk: (pre, po) for pk, m, pre, po in entries}
        if base not in d or quote not in d:
            continue
        bpre, bpost = d[base]
        qpre, qpost = d[quote]
        if bpost - bpre > 0 and qpost - qpre < 0:  # clean WSOL -> USDC
            samples.append({"cy": qpre, "ain": bpost - bpre, "aout": qpre - qpost, "t": bt or 0})
    return samples


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
    program = VENUES[venue]
    try:
        # 1. discover the dominant SOL/USDC vault pair from recent activity (program, or a market
        #    hint for venues whose program signatures are mostly oracle updates)
        found = discover_vaults(fetch_txs(url, SIG_HINT.get(venue, program), limit=70))
        if not found:
            print(f"{venue:10} no SOL/USDC pool found in recent program activity")
            return
        base, quote, _ = found
        # 2. pull pool-specific history straight from the base vault's signatures
        samples = extract_fills(fetch_txs(url, base, limit=150), base, quote)
    except Exception as e:  # noqa: BLE001
        print(f"{venue:10} ERROR: {e}")
        return
    # Restrict to a tight time window so the oracle is ~constant (otherwise drift, not model error,
    # dominates the residual). Fall back to the full set if the window is too thin.
    ts = [s["t"] for s in samples if s["t"]]
    if ts:
        tmax = max(ts)
        windowed = [s for s in samples if not s["t"] or tmax - s["t"] <= WINDOW_S]
        if sum(1 for s in windowed if s["ain"] >= DUST_LAMPORTS) >= 5:
            samples = windowed
    # Drop dust trades so the oracle inference isn't dominated by noise.
    pool = [s for s in samples if s["ain"] >= DUST_LAMPORTS]
    if len(pool) < 5:
        pool = samples
    if len(pool) < 5:
        print(f"{venue:10} only {len(samples)} clean WSOL->USDC fills (vault {base[:6]}..)")
        return
    pool.sort(key=lambda s: s["ain"])
    # Oracle = median effective price of the smallest third (robust to a single odd trade).
    k = max(1, len(pool) // 3)
    px = statistics.median(s["aout"] / s["ain"] for s in pool[:k])
    mult_x = round(px * SCALE)
    big_k, fee = fit(pool, mult_x)
    bps = sorted(abs(predict(s["ain"], s["cy"], mult_x, big_k, fee) - s["aout"]) / s["aout"] * 1e4
                 for s in pool if predict(s["ain"], s["cy"], mult_x, big_k, fee))
    pts = [s["t"] for s in pool if s["t"]]
    span = f"{max(pts) - min(pts)}s" if pts else "?"
    print(f"{venue:10} {len(pool):>3} fills/{span:>5}  ~{px * 1000:6.2f} USDC/SOL  "
          f"sizes {pool[0]['ain'] / 1e9:6.3f}-{pool[-1]['ain'] / 1e9:<7.3f} SOL  "
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
        run(url, v) if v in VENUES else print(f"{v:10} unknown (have: {', '.join(VENUES)})")


if __name__ == "__main__":
    main()
