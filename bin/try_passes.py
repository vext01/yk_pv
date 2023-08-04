#!/usr/bin/env python3

import os, sys, random
from subprocess import Popen, PIPE, TimeoutExpired
from dataclasses import dataclass

# Stages in an LTO pipeline where optimisation passes can happen.
STAGES = "pre_link", "link_time"

# Max time to wait for one pipeline test to run.
TIMEOUT = 60 * 10

@dataclass
class Pass:
    name: str
    # FIXME: implement support for pass parameters?
    #args: list

    def __str__(self):
        return self.name

@dataclass
class PipelineConfig:
    pre_link: list
    link_time: list

    def __str__(self):
        pre_link = ",".join([ str(p) for p in self.pre_link ])
        link_time = ",".join([ str(p) for p in self.link_time ])
        return f"PipelineConfig(pre_link=[{pre_link}], link_time=[{link_time}])"

def get_all_passes():
    p = Popen("opt --print-passes | grep -v ':$'", shell=True, stdout=PIPE,
              close_fds=True)
    sout, serr = p.communicate()
    assert(p.returncode == 0)
    sout = sout.decode()
    pass_descrs = [x.strip() for x in sout.strip().split("\n")]

    passes = []
    for descr in pass_descrs:
        # strip off parameters for now.
        parts = descr.split("<")
        passes.append(Pass(parts[0]))

    print(f"Found {len(passes)} passes")
    return passes


def log(logf, s):
    logf.write(s)
    logf.flush()


def test_pipeline(logf, pl):
    sys.stdout.write(str(pl) + "...")
    sys.stdout.flush()

    log(logf, "\n\n" + str(pl) + "\n")

    env = os.environ
    env["PRELINK_PASSES"] = ",".join([p.name for p in pl.pre_link])
    env["LINKTIME_PASSES"] = ",".join([p.name for p in pl.link_time])

    p = Popen("sh test.sh 2>&1", cwd="/home/vext01/research/yklua", shell=True,
              stdout=PIPE, close_fds=True, env=env)
    try:
        sout, _ =  p.communicate(timeout=TIMEOUT)
    except TimeoutExpired:
        log(logf, "!!! TIMED OUT !!!\n")
    else:
        log(logf, sout.decode())

    if p.returncode == 0:
        print(" [OK]")
        log(logf, str(pl) + ": OK\n")
    else:
        log(logf, str(pl) + " : FAILED\n")
        print(" [FAIL]")
    return p.returncode == 0


def main(logf):
    # sanity check, test script should work with no extra passes.
    #assert(test_pipeline(logf, PipelineConfig([], [])))

    passes = get_all_passes()
    #attempt1(logf, passes)
    attempt2(logf, passes)


# Try each pass in isolation, first in the pre-link pipeline, then in the
# link-time pipeline.
#
# By my calculations, this would take about 36 hours, and that's without
# repeating any tests for added confidence.
def attempt1(logf, passes):
    # First try each pass in isolation and prune those that fail away
    # immediatley.
    results = { s: { False: [], True: [] } for s in STAGES }
    for stage in STAGES:
        for pss in passes:
            if stage == "pre_link":
                pl = PipelineConfig([pss], [])
            else:
                pl = PipelineConfig([], [pss])
            results[stage][test_pipeline(logf, pl)].append(pss)

    for stage in STAGES:
        print(f"\n\nResults for passes in isolation for stage: {stage}")
        print(72 * "=")
        ok = ",".join([str(p) for p in results[stage][True]])
        fail = ",".join([str(p) for p in results[stage][False]])

        print(f"\nOK: ")
        print(72 * '-')
        print(ok)

        print(f"\nFAIL: ")
        print(72 * '-')
        print(fail)


def list_of_passes_to_str(passes):
    return ",".join([str(p) for p in passes])


def attempt2_inner(logf, ok_passes, try_passes):
    log(logf, f"\n>>> OK passes so far:\n{list_of_passes_to_str(ok_passes)}\n")
    log(logf, f">>> Trying to add:\n{list_of_passes_to_str(try_passes)}\n\n")

    # Use the same pipeline for both stages for now.
    if test_pipeline(logf, PipelineConfig(ok_passes + try_passes, ok_passes + try_passes)):
        ok_passes.extend(try_passes)
    elif len(try_passes) == 1:
        return
    else:
        random.shuffle(try_passes)
        subset1 = try_passes[:len(try_passes) // 2]
        subset2 = try_passes[len(try_passes) // 2:]
        # XXX: We assume that whatever subsets we accept from this splitting
        # work in combination, but that may not be true. Not sure what we'd do
        # if they didn't work in combination though... restart the whole
        # search?
        attempt2_inner(logf, ok_passes, subset1)
        attempt2_inner(logf, ok_passes, subset2)


# Use a binary tree to attempt to speed things up by accepting whole groups of
# OK passes at once.
def attempt2(logf, passes):
    ok_passes = []
    random.shuffle(passes)
    attempt2_inner(logf, ok_passes, passes)

    print("\n\nFinal OK passes")
    print(72 * "=")
    print(list_of_passes_to_str(ok_passes))


if __name__ == "__main__":
    with open("passes.log", "w") as logf:
        main(logf)
