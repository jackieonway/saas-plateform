#!/usr/bin/env python3
"""Convert the Navicat t_region dump into deterministic PostgreSQL COPY assets.

Usage: python backend/assets/prepare-region-seed.py [--inspect]
Only the fields used by the region catalog are retained; the source is never
executed as SQL. Assets are compressed and embedded by the Rust bootstrap.
"""

from __future__ import annotations

import argparse
import csv
import gzip
import io
from collections import Counter
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOURCE = ROOT / "docs" / "t_region.sql"
OUTPUT = ROOT / "assets"
PREFIX = "INSERT INTO `t_region` VALUES ("


def copy_field(value: str) -> str:
    if value == "NULL":
        return r"\N"
    return value.replace("\\", r"\\").replace("\t", r"\t").replace("\n", r"\n").replace("\r", r"\r")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--inspect", action="store_true", help="count source records without writing assets")
    args = parser.parse_args()

    writers: dict[str, tuple[io.TextIOWrapper, gzip.GzipFile, io.BufferedWriter]] = {}
    years: Counter[str] = Counter()
    levels: Counter[str] = Counter()
    seen_codes: dict[str, dict[str, tuple[str, ...]]] = {}
    parent_codes: dict[str, set[str]] = {}
    duplicate_codes = 0
    duplicate_examples: list[tuple[str, str, str]] = []
    conflicting_examples: list[tuple[str, str, str]] = []
    conflicting_codes = 0
    deleted = 0
    try:
        with SOURCE.open("r", encoding="utf-8-sig", newline="") as source:
            for line_number, line in enumerate(source, 1):
                if not line.startswith(PREFIX):
                    continue
                if not line.rstrip().endswith(");"):
                    raise ValueError(f"line {line_number}: incomplete INSERT")
                values = line[len(PREFIX):line.rfind(");")]
                row = next(csv.reader([values], delimiter=",", quotechar="'", escapechar="\\", skipinitialspace=True))
                if len(row) != 19:
                    raise ValueError(f"line {line_number}: expected 19 fields, got {len(row)}")
                if row[18] != "0":
                    deleted += 1
                    continue
                year = row[11]
                if not (len(year) == 4 and year.isdigit()):
                    raise ValueError(f"line {line_number}: invalid year")
                codes = seen_codes.setdefault(year, {})
                if row[1] in codes:
                    duplicate_codes += 1
                    if len(duplicate_examples) < 10:
                        duplicate_examples.append((row[1], row[2], row[3]))
                    if codes[row[1]] != tuple(row[2:11]):
                        conflicting_codes += 1
                        if len(conflicting_examples) < 10:
                            conflicting_examples.append((row[1], row[2], row[3]))
                    continue
                else:
                    codes[row[1]] = tuple(row[2:11])
                years[year] += 1
                levels[row[6]] += 1
                parent_codes.setdefault(year, set()).add(row[2])
                widths = (32, 12, 12, 128, 512, 64, 0, 12, 32, 512, 512)
                for index, maximum in enumerate(widths):
                    if maximum and row[index] != "NULL" and len(row[index]) > maximum:
                        raise ValueError(f"line {line_number}: field {index} exceeds {maximum} characters")
                if args.inspect:
                    continue
                if year not in writers:
                    OUTPUT.mkdir(parents=True, exist_ok=True)
                    file = (OUTPUT / f"regions-{year}.copy.gz").open("wb")
                    compressed = gzip.GzipFile(fileobj=file, mode="wb", filename="", mtime=0, compresslevel=9)
                    text = io.TextIOWrapper(compressed, encoding="utf-8", newline="\n")
                    writers[year] = (text, compressed, file)
                # version, source_id, code, parent_code, name, pinyin, short,
                # level, type, type_name, post_code, source_url.
                fields = [year, row[0], *row[1:11]]
                writers[year][0].write("\t".join(copy_field(value) for value in fields) + "\n")
    finally:
        for text, compressed, file in writers.values():
            text.close()
            compressed.close()
            file.close()
    orphans = {year: sorted(parents - codes - {"-1"})[:5]
               for year, parents in parent_codes.items() for codes in [set(seen_codes[year])] if parents - codes - {"-1"}}
    if orphans or conflicting_codes:
        raise ValueError(f"missing parent examples={orphans}, conflicting codes={conflicting_examples}")
    print(f"rows={sum(years.values())} years={dict(years)} levels={dict(levels)} deleted={deleted} duplicate_codes={duplicate_codes} conflicting_codes={conflicting_codes} examples={duplicate_examples} conflicts={conflicting_examples}")
    if not args.inspect:
        for year in sorted(years):
            path = OUTPUT / f"regions-{year}.copy.gz"
            print(f"{path.relative_to(ROOT)} bytes={path.stat().st_size}")


if __name__ == "__main__":
    main()
