#!/usr/bin/env python3

from collections import defaultdict
import os
import re
import sys

TRACE_FUNC_RE = re.compile('__yk_trace_[0-9]+')

def is_opt(yk_outlined, fn):
    #return fn.startswith("__yk_opt")
    return fn in yk_outlined and (not fn == "luaV_execute")

def is_tc(fl, fn):
    # ^ just a heuristic
    return fl.endswith(".rs") or "::" in fn or "rust" in fn

def is_trace(fn):
    return TRACE_FUNC_RE.match(fn) is not None

def process_file(yk_outlined, f):
    header = True
    fl = None
    fn = None
    summary_count = None
    func_events = defaultdict(int)
    total_events = 0
    trace_events = 0
    opt_events = 0
    tcompiler_events = 0
    for line in f:
        line = line.strip()
        if line.startswith("fl="):
            header = False

        if line.startswith("summary"):
            k, v = line.split(":")
            summary_count = int(v)
            continue

        if header:
            elems = line.split(":")
            assert(len(elems) == 2)
            if elems[0] == "events":
                elem = elems[1].strip()
                assert elem == "Ir"
        else:
            if line.startswith("fl="):
                k, fl = line.split("=")
                fn = None
            elif line.startswith("fn="):
                k, fn = line.split("=", 1)
            else:
                lineno, val = line.split(" ")
                val = int(val)
                total_events += val
                func_events[(fl, fn)] += val
                if is_tc(fl, fn):
                    tcompiler_events += val
                elif is_opt(yk_outlined, fn):
                    opt_events += val
                elif is_trace(fn):
                    trace_events += val

    assert(summary_count == total_events)
    return {
            "total_events": total_events,
            "tcompiler_events": tcompiler_events,
            "trace_events": trace_events,
            "opt_events": opt_events,
            "func_events": func_events,
            }

def c_tracing_perc(d):
    total_events_notc = d["total_events"] - d["tcompiler_events"]
    return d["trace_events"] / total_events_notc * 100

def c_opt_perc(d):
    total_events_notc = d["total_events"] - d["tcompiler_events"]
    return d["opt_events"] / total_events_notc * 100

def mode_summary(yk_outlined, files):
    data = {}
    for fname in files:
        print(">> " + fname)
        with open(fname) as f:
            data[fname] = process_file(yk_outlined, f)

    sorted_data = sorted(data.items(), key=lambda item: c_tracing_perc(item[1]))

    hdr_file = "file"
    hdr_trace = "%trace"
    hdr_opt = "%opt"
    hdr_other = "%other"
    print("note: excludes events in functions that look like the trace compiler")
    print("\n")
    print(f"{hdr_file:30}  {hdr_trace:6}     {hdr_opt:6}       {hdr_other:6}")
    print("-" * 73)
    for fname, data in sorted_data:
        tracing_perc = c_tracing_perc(data)
        opt_perc = c_opt_perc(data)
        other_perc = 100 - (tracing_perc + opt_perc)
        assert(99.9 <= (tracing_perc + opt_perc + other_perc) <= 100.1)
        print(f"{fname:30} {tracing_perc:6.2f}%    {opt_perc:6.2f}%" + \
                f"      {other_perc:6.2f}%")

def mode_makeup(yk_outlined, file):
    data = None
    with open(file) as f:
        data = process_file(yk_outlined, f)

    traces_perc = c_tracing_perc(data)
    opt_perc = c_opt_perc(data)
    other_perc = 100 - (traces_perc + opt_perc)
    assert(99.9 <= (traces_perc + opt_perc + other_perc) <= 100.1)

    makeup = {
            "traces": (traces_perc, data["trace_events"], []),
            "opt": (opt_perc, data["opt_events"], []),
            "other": (other_perc, data["total_events"] - \
                    data["tcompiler_events"] - data["trace_events"] - \
                    data["opt_events"], []),
            }

    for (fl, fn), val in data["func_events"].items():
        r = None
        if is_tc(fl, fn):
            continue
        elif is_trace(fn):
            r = makeup["traces"]
        elif is_opt(yk_outlined, fn):
            r = makeup["opt"]
        else:
            r = makeup["other"]

        perc = val / r[1] * 100
        r[2].append(((fl, fn), perc))

    for kind, (all_perc, all, funcs) in makeup.items():
        print(f">> {kind} ({all_perc:6.2f}%): ")
        funcs = sorted(funcs, key=lambda f: f[1], reverse=True)
        sum_perc = 0
        for (fl, fn), perc in funcs:
            fl_b = os.path.basename(fl)
            print(f"  {fl_b:30} {fn:30} {perc:6.2f}%")
            sum_perc += perc
        assert(99.9 <= sum_perc <= 100.1)


def read_yk_outlined():
    yk_outlined = set()
    with open("OUTLINEMAP") as f:
        for line in f:
            yk_outlined.add(line.strip())
    return yk_outlined

if __name__ == "__main__":
    yk_outlined = read_yk_outlined()
    if sys.argv[1] == "summary":
        mode_summary(yk_outlined, sys.argv[2:])
    elif sys.argv[1] == "makeup":
        mode_makeup(yk_outlined, sys.argv[2])
    else:
        print("bad usage")
