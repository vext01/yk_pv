#!/usr/bin/env python3

from collections import defaultdict
import re
import sys

TRACE_FUNC_RE = re.compile('__yk_trace_[0-9]+')

def process_file(fl):
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
                func_events[fn] += val
                if fl.endswith(".rs") or "::" in fn or "rust" in fn:
                    # ^ just a heuristic
                    # do a release-with-debug build and we can probably filter
                    # just by filename?
                    tcompiler_events += val
                elif fn.startswith("__yk_opt_"):
                    opt_events += val
                elif TRACE_FUNC_RE.match(fn):
                    trace_events += val

    assert(summary_count == total_events)
    return {
            "total_events": total_events,
            "tcompiler_events": tcompiler_events,
            "trace_events": trace_events,
            "opt_events": opt_events,
            "func-events": func_events,
            }

if __name__ == "__main__":
    data = {}
    for fname in sys.argv[1:]:
        print(">> " + fname)
        with open(fname) as f:
            data[fname] = process_file(f)

    def c_tcomp_perc(d):
        return d["tcompiler_events"] / d["total_events"] * 100

    def c_tracing_perc(d):
        return d["trace_events"] / d["total_events"] * 100

    def c_opt_perc(d):
        return d["opt_events"] / d["total_events"] * 100

    sorted_data = sorted(data.items(), key=lambda item: c_tracing_perc(item[1]))

    hdr_file = "file"
    hdr_trace = "%trace"
    hdr_opt = "%__yk_opt_*"
    hdr_tcomp = "%tcomp"
    hdr_other = "%other"
    print(f"{hdr_file:30}  {hdr_trace:6}     {hdr_opt:6}   {hdr_tcomp:6}    {hdr_other:6}")
    print("-" * 73)
    for fname, data in sorted_data:
        tracing_perc = c_tracing_perc(data)
        opt_perc = c_opt_perc(data)
        tcomp_perc = c_tcomp_perc(data)
        other_perc = 100 - (tracing_perc + opt_perc + tcomp_perc)
        print(f"{fname:30} {tracing_perc:6.2f}%    {opt_perc:6.2f}%" + \
                f"      {tcomp_perc:6.2f}%    {other_perc:6.2f}%")
