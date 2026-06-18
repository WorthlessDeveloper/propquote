#!/usr/bin/env python3
"""Separate each prop AMM's spread from its oracle, using two-sided fills + an external Pyth feed.

The earlier fitter folded spread into the inferred oracle. This fixes that: by collecting BOTH
directions (WSOL->USDC sells and USDC->WSOL buys), the near-mid sell price (bid) and buy price (ask)
straddle the oracle mid, so their gap is the venue's spread — no oracle guess needed. We also pull
an independent Pyth SOL/USD price and report how far each venue's mid sits from it (stale/skew check).

Usage:
    SOLANA_RPC_URL=<rpc> python tools/spread_probe.py [venue ...]
"""
import json
import os
import statistics
import sys
import urllib.request
from collections import Counter

WSOL = "So11111111111111111111111111111111111111112"
USDC = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v"
WINDOW_S = 45
PYTH_SOL_USD = "ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d"

VENUES = {
    "solfi-v2": "SV2EYYJyRz2YhfXwXnhNAevDEui5Q6yrfyo13WtupPF",
    "zerofi": "ZERor4xhbUycZ6gb9ntrhqscUcZmAbQDjEAtCf4hbZY",
    "tessera": "TessVdML9pBGgG9yGks7o4HewRaXVAMuoVj4x83GLQH",
    "humidifi": "9H6tua7jkLhdm3w8BvgpTn5LZNU7g4ZynDmCiNN3q6Rp",
    "bisonfi": "BiSoNHVpsVZW2F7rx2eQ59yQwKxzU5NvBcmKshCSUypi",
}
SIG_HINT = {"humidifi": "FksffEqnBRixYGR791Qw2MgdU7zNCpHVFYBL4Fa4qVuH"}


def rpc(url, method, params):
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    return json.load(urllib.request.urlopen(req, timeout=40)).get("result")


def pyth_sol_usd():
    try:
        u = f"https://hermes.pyth.network/v2/updates/price/latest?ids[]={PYTH_SOL_USD}"
        d = json.load(urllib.request.urlopen(u, timeout=20))
        p = d["parsed"][0]["price"]
        return int(p["price"]) * (10 ** int(p["expo"]))
    except Exception:  # noqa: BLE001
        return None


def fetch_txs(url, addr, limit=150):
    sigs = rpc(url, "getSignaturesForAddress", [addr, {"limit": limit}]) or []
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
        comb = ([k["pubkey"] for k in msg.get("accountKeys", [])]
                + loaded.get("writable", []) + loaded.get("readonly", []))
        post = {b["accountIndex"]: int(b["uiTokenAmount"]["amount"]) for b in meta.get("postTokenBalances") or []}
        entries = []
        for b in meta.get("preTokenBalances") or []:
            i = b["accountIndex"]
            if i < len(comb):
                pre = int(b["uiTokenAmount"]["amount"])
                entries.append((comb[i], b.get("mint"), pre, post.get(i, pre)))
        out.append((entries, tx.get("blockTime")))
    return out


def discover_vaults(txs):
    pair = Counter()
    for entries, _ in txs:
        ws = {pk for pk, m, pre, po in entries if m == WSOL and pre != po}
        us = {pk for pk, m, pre, po in entries if m == USDC and pre != po}
        for w in ws:
            for u in us:
                pair[(w, u)] += 1
    return pair.most_common(1)[0][0] if pair else None


def two_sided(txs, base, quote):
    """Return (sells, buys): sells=(sol_in, usdc_out), buys=(usdc_in, sol_out), with blockTime."""
    sells, buys = [], []
    for entries, bt in txs:
        d = {pk: (pre, po) for pk, m, pre, po in entries}
        if base not in d or quote not in d:
            continue
        bd = d[base][1] - d[base][0]   # WSOL vault delta
        qd = d[quote][1] - d[quote][0]  # USDC vault delta
        if bd > 0 and qd < 0:
            sells.append((bd, -qd, bt or 0))      # sold SOL -> got USDC
        elif bd < 0 and qd > 0:
            buys.append((qd, -bd, bt or 0))        # paid USDC -> got SOL
    return sells, buys


def tight(rows):
    ts = [r[2] for r in rows if r[2]]
    if not ts:
        return rows
    tmax = max(ts)
    w = [r for r in rows if not r[2] or tmax - r[2] <= WINDOW_S]
    return w if len(w) >= 3 else rows


def near_mid_price(rows, sol_idx, usdc_idx):
    """Median price (USDC/SOL) of the smallest-SOL third — least price impact, ~ the quote at mid."""
    rows = sorted(rows, key=lambda r: r[sol_idx])
    k = max(1, len(rows) // 3)
    return statistics.median((r[usdc_idx] / 1e6) / (r[sol_idx] / 1e9) for r in rows[:k])


def run(url, venue, ext):
    program = VENUES[venue]
    try:
        found = discover_vaults(fetch_txs(url, SIG_HINT.get(venue, program), limit=70))
        if not found:
            print(f"{venue:10} no SOL/USDC pool found")
            return
        base, quote = found
        sells, buys = two_sided(fetch_txs(url, base, limit=200), base, quote)
    except Exception as e:  # noqa: BLE001
        print(f"{venue:10} ERROR: {e}")
        return
    sells, buys = tight(sells), tight(buys)
    if len(sells) < 3 or len(buys) < 3:
        print(f"{venue:10} one-sided: {len(sells)} sells / {len(buys)} buys — need both for spread")
        return
    bid = near_mid_price(sells, 0, 1)   # USDC received per SOL sold
    ask = near_mid_price(buys, 1, 0)    # USDC paid per SOL bought
    mid = (bid + ask) / 2
    spread_bps = (ask - bid) / mid * 1e4
    ext_str = f"{(mid - ext) / ext * 1e4:+7.1f}" if ext else "    n/a"
    print(f"{venue:10} sells {len(sells):>3} / buys {len(buys):>3}   "
          f"bid {bid:7.2f}  ask {ask:7.2f}  mid {mid:7.2f}   "
          f"spread {spread_bps:6.1f} bps   vs Pyth {ext_str} bps")


def main():
    args = sys.argv[1:]
    url = os.environ.get("SOLANA_RPC_URL")
    if args and "://" in args[0]:
        url = args.pop(0)
    if not url:
        sys.exit("set SOLANA_RPC_URL or pass the RPC url first")
    ext = pyth_sol_usd()
    print(f"external reference (Pyth SOL/USD): {ext:.2f}" if ext else "Pyth feed unavailable")
    print(f"{'venue':10} {'fills':>15}   {'bid/ask/mid (USDC/SOL)':>34}   spread       skew-vs-pyth")
    print("-" * 104)
    for v in (args or VENUES):
        run(url, v, ext) if v in VENUES else print(f"{v:10} unknown")


if __name__ == "__main__":
    main()
