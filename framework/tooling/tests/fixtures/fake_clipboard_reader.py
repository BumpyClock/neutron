#!/usr/bin/env python3
"""Synthetic independent clipboard reader for harness regression tests."""

import sys


sys.stdout.buffer.write(b"synthetic clipboard payload")
