#!/bin/sh

set -eu

ITERS=10
YKLUA=~/research/yklua/src/lua
VALGRIND="/home/vext01/source/valgrind/inst/bin/valgrind"
HARNESS="../../awfy/Lua/harness.lua"
TOPDIR=$(pwd)

export YKD_SERIALISE_COMPILATION=1
export LUA_PATH="?.lua;../../awfy/Lua/?.lua"

# dir, benchmark, param
run_cachegrind() {
    DIR=$1; shift
    BENCH=$1; shift
    PARAM=$1; shift
    cd $TOPDIR/$DIR
    set +e
    $VALGRIND --tool=cachegrind --cachegrind-out-file=cachegrind.$BENCH.out \
        $YKLUA $HARNESS $BENCH $ITERS $PARAM > $BENCH.log 2>&1 &
    set -e
}

AWFY_DIR=suites/awfy/Lua
run_cachegrind $AWFY_DIR deltablue 12000
run_cachegrind $AWFY_DIR richards 100
run_cachegrind $AWFY_DIR json 100
run_cachegrind $AWFY_DIR cd 250
run_cachegrind $AWFY_DIR havlak 1500
run_cachegrind $AWFY_DIR bounce 1500
run_cachegrind $AWFY_DIR list 1500
run_cachegrind $AWFY_DIR mandelbrot 500
run_cachegrind $AWFY_DIR nbody 250000
run_cachegrind $AWFY_DIR permute 1000
run_cachegrind $AWFY_DIR queens 1000
run_cachegrind $AWFY_DIR sieve 3000
run_cachegrind $AWFY_DIR storage 1000
run_cachegrind $AWFY_DIR towers 600

YK_DIR=suites/yk/Lua
run_cachegrind $YK_DIR bigloop 1000000000

RW_DIR=suites/realworld/Lua
run_cachegrind $RW_DIR lulpeg x
run_cachegrind $RW_DIR hashids 6000
run_cachegrind $RW_DIR heightmap 2000

CL_DIR=suites/cbgame/Lua
run_cachegrind $CL_DIR fannkuchredux 10
run_cachegrind $CL_DIR spectralnorm 1000
run_cachegrind $CL_DIR fasta 500000
run_cachegrind $CL_DIR knucleotide x
run_cachegrind $CL_DIR revcomp x
run_cachegrind $CL_DIR binarytrees 15
