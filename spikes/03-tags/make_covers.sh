#!/bin/sh
# Generates the two test cover JPEGs from ffmpeg test patterns plus noise
# (no third-party images). Run from spikes/03-tags.
set -e
mkdir -p work
ffmpeg -v error -y -f lavfi -i "testsrc2=s=800x800,noise=alls=40:allf=t+u" -frames:v 1 -q:v 3 work/cover_300k.jpg
ffmpeg -v error -y -f lavfi -i "testsrc2=s=2600x2600,noise=alls=70:allf=t+u" -frames:v 1 -q:v 1 work/cover_5m.jpg
