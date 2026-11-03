#!/usr/bin/env python3
"""Prepare inert marketplace artifacts with CadencR's canonical Rust packer."""

import argparse
import hashlib
import json
import re
import shutil
import subprocess
import tarfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGETS = {
    "darwin-aarch64", "darwin-x86_64", "linux-aarch64", "linux-x86_64",
}


def read_json(path):
    return json.loads(path.read_text())


def write_json(path, value):
    with path.open("x") as output:
        output.write(json.dumps(value, indent=2) + "\n")


def run(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True, timeout=120).strip()


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def identity():
    package = read_json(ROOT / "marketplace/package-template.json")
    descriptor = read_json(ROOT / "descriptor/pi-rust.json")
    version = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
    if not re.fullmatch(r"\d+\.\d+\.\d+", version):
        raise ValueError("Release version must be an exact stable semantic version")
    for key in ("id", "name", "version", "repository", "license"):
        if package["agent"][key] != descriptor["agent"][key]:
            raise ValueError(f"Descriptor and marketplace identity differ: {key}")
    if package["agent"]["version"] != version:
        raise ValueError("Cargo and marketplace versions differ")
    return package, descriptor


def prepare(args):
    package, descriptor = identity()
    binary = args.binary.resolve(strict=True)
    if run(str(binary), "version") != package["agent"]["version"]:
        raise ValueError("Executable version differs from the release metadata")
    args.output.mkdir(parents=True, exist_ok=False)
    stage = args.output / "staging"
    (stage / "bin").mkdir(parents=True)
    shutil.copyfile(binary, stage / "bin/provider")
    (stage / "bin/provider").chmod(0o755)
    for name in ("README.md", "LICENSE", "THIRD_PARTY_NOTICES.md", "icon.svg"):
        shutil.copyfile(ROOT / name, stage / name)
        (stage / name).chmod(0o644)
    descriptor["installation"]["executable"] = {"command": str(stage / "bin/provider")}
    descriptor["installation"]["assets"] = {"directory": str(stage)}
    descriptor_path = args.output / "pi-rust.json"
    write_json(descriptor_path, descriptor)
    run(str(args.cli), "plugin", "validate", str(stage), "--descriptor", str(descriptor_path))
    version = package["agent"]["version"]
    name = f"provider-pi-rust-{version}-{args.target}.tar.gz"
    package["agent"]["distribution"]["binary"] = {
        args.target: {
            "archive": f'{package["agent"]["repository"]}/releases/download/v{version}/{name}',
            "cmd": "bin/provider",
            "sha256": "0" * 64,
        }
    }
    input_path = args.output / "pack-input.json"
    write_json(input_path, package)
    archive = args.output / name
    receipt = json.loads(run(
        str(args.cli), "registry", "pack-provider", "--package", str(input_path),
        "--target", args.target, "--directory", str(stage), "--output", str(archive),
    ))
    repeated = args.output / "reproducibility.tar.gz"
    run(
        str(args.cli), "registry", "pack-provider", "--package", str(input_path),
        "--target", args.target, "--directory", str(stage), "--output", str(repeated),
    )
    if digest(archive) != receipt["sha256"] or digest(repeated) != receipt["sha256"]:
        raise ValueError("Archive digest or reproducibility check failed")
    if archive.stat().st_size != receipt["size"]:
        raise ValueError("Archive size differs from the packer receipt")
    package["agent"]["distribution"]["binary"][args.target]["sha256"] = receipt["sha256"]
    package_path = args.output / "package.json"
    write_json(package_path, package)
    run(str(args.cli), "registry", "validate-metadata", str(package_path))
    extracted = args.output / "extracted"
    extracted.mkdir()
    with tarfile.open(archive) as payload:
        payload.extractall(extracted, filter="data")
    write_json(args.output / "pack-receipt.json", receipt)
    print(json.dumps({"archive": str(archive), "extracted": str(extracted), "sha256": receipt["sha256"]}))


def assemble(args):
    package, _ = identity()
    if len(args.targets) != len(set(args.targets)):
        raise ValueError("Release targets must be unique")
    version = package["agent"]["version"]
    commit = run("git", "rev-parse", "HEAD", cwd=ROOT)
    if args.source_commit != commit or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise ValueError("Submission source must be the exact local HEAD commit")
    if run("git", "status", "--porcelain", cwd=ROOT):
        raise ValueError("Submission requires a clean source worktree")
    if not args.changelog.read_text().strip():
        raise ValueError("A release changelog is required")
    args.output.mkdir(parents=True, exist_ok=False)
    evidence = []
    sums = []
    for target in sorted(args.targets):
        folder = args.artifacts / target
        candidate = read_json(folder / "package.json")
        native = read_json(folder / "native-qa.json")
        if native["target"] != target or native["passed"] is not True or native["pi_version"] != "0.85.1":
            raise ValueError(f"Missing native qualification for {target}")
        comparison = json.loads(json.dumps(candidate))
        comparison["agent"]["distribution"]["binary"] = {}
        expected = json.loads(json.dumps(package))
        expected["agent"]["distribution"]["binary"] = {}
        if comparison != expected or set(candidate["agent"]["distribution"]["binary"]) != {target}:
            raise ValueError(f"Inconsistent package metadata for {target}")
        entry = candidate["agent"]["distribution"]["binary"][target]
        archive_name = f"provider-pi-rust-{version}-{target}.tar.gz"
        archive = folder / archive_name
        if digest(archive) != entry["sha256"]:
            raise ValueError(f"Archive integrity failed for {target}")
        with tarfile.open(archive) as payload:
            with payload.extractfile("bin/provider") as executable:
                executable_sha = hashlib.file_digest(executable, "sha256").hexdigest()
        if native["version"] != version or native["provider_sha256"] != executable_sha:
            raise ValueError(f"Native evidence does not match the packaged executable for {target}")
        expected_url = f'{package["agent"]["repository"]}/releases/download/v{version}/{archive_name}'
        if entry["archive"] != expected_url or entry["cmd"] != "bin/provider":
            raise ValueError(f"Unexpected release asset binding for {target}")
        package["agent"]["distribution"]["binary"][target] = entry
        shutil.copyfile(archive, args.output / archive_name)
        sums.append(f'{entry["sha256"]}  {archive_name}\n')
        evidence.append(native)
    submission = {
        "schema_version": 1,
        "package": package,
        "source": {
            "repository": package["agent"]["repository"], "commit": commit, "tag": f"v{version}",
        },
        "changelog": args.changelog.read_text(),
    }
    package_path = args.output / "package.json"
    submission_path = args.output / "submission.json"
    write_json(package_path, package)
    write_json(submission_path, submission)
    write_json(args.output / "native-qa.json", evidence)
    run(str(args.cli), "registry", "validate-metadata", str(package_path))
    run(str(args.cli), "registry", "validate-submission", str(submission_path))
    (args.output / "SHA256SUMS").write_text("".join(sums))
    shutil.copyfile(args.changelog, args.output / "RELEASE_NOTES.md")
    print(f"Prepared pi-rust {version}: {', '.join(sorted(args.targets))}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    package = commands.add_parser("package")
    package.add_argument("--target", choices=sorted(TARGETS), required=True)
    package.add_argument("--binary", type=Path, required=True)
    package.set_defaults(action=prepare)
    release = commands.add_parser("assemble")
    release.add_argument("--artifacts", type=Path, required=True)
    release.add_argument("--targets", choices=sorted(TARGETS), nargs="+", required=True)
    release.add_argument("--source-commit", required=True)
    release.add_argument("--changelog", type=Path, required=True)
    release.set_defaults(action=assemble)
    for command in (package, release):
        command.add_argument("--cli", type=Path, required=True)
        command.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.cli = args.cli.resolve(strict=True)
    args.output = args.output.resolve()
    args.action(args)


if __name__ == "__main__":
    main()
