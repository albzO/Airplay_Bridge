"""Collect the license and notice files of the resolved Windows Rust dependencies."""

import csv
import json
import re
import shutil
import sys
from pathlib import Path


NOTICE_NAME = re.compile(r"^(license|licence|copying|notice|copyright|unlicense)([._-]|$)", re.I)


def runtime_packages(metadata):
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    pending = [metadata["resolve"]["root"]]
    seen = set()
    while pending:
        package_id = pending.pop()
        if package_id in seen:
            continue
        seen.add(package_id)
        pending.extend(
            edge["pkg"]
            for edge in nodes[package_id]["deps"]
            if any(kind["kind"] is None for kind in edge["dep_kinds"])
        )
    return sorted(
        (p for p in metadata["packages"] if p["id"] in seen and p["source"]),
        key=lambda p: (p["name"], p["version"]),
    )


def notice_files(package, root):
    candidates = {path for path in root.iterdir() if NOTICE_NAME.match(path.name)}
    candidates.update((root / "LICENSES").glob("*"))
    if package["license_file"]:
        candidates.add(root / package["license_file"])
    for candidate in sorted(candidates):
        resolved = candidate.resolve()
        if resolved.is_relative_to(root) and resolved.is_file():
            yield resolved


def collect(metadata_path, destination):
    metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
    destination.mkdir(parents=True, exist_ok=True)
    with (destination / "DEPENDENCIES.tsv").open("w", encoding="utf-8", newline="") as index:
        writer = csv.writer(index, delimiter="\t", lineterminator="\n")
        writer.writerow(("name", "version", "declared_license", "bundled_notice_files"))
        for package in runtime_packages(metadata):
            root = Path(package["manifest_path"]).resolve().parent
            package_dir = destination / f"{package['name']}-{package['version']}"
            files = []
            for source in notice_files(package, root):
                relative = source.relative_to(root)
                target = package_dir / relative
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(source, target)
                files.append(target.relative_to(destination).as_posix())
            writer.writerow((package["name"], package["version"], package["license"] or "", ", ".join(files)))


if __name__ == "__main__":
    collect(Path(sys.argv[1]), Path(sys.argv[2]))
