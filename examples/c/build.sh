#!/bin/bash

OUTPUT_LIB=../../target/debug/
HEADER_PATH=../../

clang -o example main.c -I $HEADER_PATH -L $OUTPUT_LIB -l pitaya