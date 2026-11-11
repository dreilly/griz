#!/usr/bin/env python3
"""Build a self-contained Apple Silicon tester zip. No updater or installer."""
import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parent.parent
TARGET = "aarch64-apple-darwin"
BUNDLE_ID = "io.github.dreilly.griz"


def run(*args, **kwargs):
    return subprocess.run(args, cwd=ROOT, check=True, **kwargs)


def output(*args):
    return run(*args, stdout=subprocess.PIPE, text=True).stdout.strip()


def revision(path):
    commit = output("git", "-C", str(path), "rev-parse", "HEAD")
    dirty = bool(output("git", "-C", str(path), "status", "--porcelain", "--untracked-files=normal"))
    return commit, dirty


def check_binary(path):
    archs = output("lipo", "-archs", str(path)).split()
    if archs != ["arm64"]:
        raise RuntimeError(f"{path.name} must be arm64 only, found {archs}")
    links = output("otool", "-L", str(path))
    for line in links.splitlines()[1:]:
        dependency = line.strip().split(" (", 1)[0]
        if not dependency.startswith(("/System/Library/", "/usr/lib/")):
            raise RuntimeError(f"Unbundled runtime dependency: {dependency}")
    return links


def dependency_materials(metadata, resources):
    """Keep notices and locked sources with the app, including MPL source code."""
    notices = resources / "ThirdPartyNotices"
    notices.mkdir()
    shutil.copytree(ROOT / "assets/licenses", notices / "Supplemental")
    shutil.copy(ROOT / "assets/harnesses/NOTICE.md", notices / "Harness-logos.md")
    index = ["Griz third-party dependency inventory", "",
             "Includes build/test dependencies as well as runtime dependencies.",
             "Declared authors/licenses below are from Cargo metadata.",
             "Original locked source packages are in ThirdPartySources.zip.", ""]
    with zipfile.ZipFile(resources / "ThirdPartySources.zip", "w", zipfile.ZIP_DEFLATED) as sources:
        for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
            if package["id"] in metadata["workspace_members"]:
                continue
            name = f"{package['name']}-{package['version']}"
            directory = Path(package["manifest_path"]).parent
            index.extend([name, f"License: {package.get('license') or 'See source'}",
                          f"Authors: {', '.join(package.get('authors', []))}",
                          f"Repository: {package.get('repository') or 'Local source'}", ""])
            destination = notices / name
            destination.mkdir()
            candidates = [p for p in directory.iterdir()
                          if p.name.lower().startswith(("license", "licence", "copying", "notice", "copyright"))]
            if package.get("license_file"):
                candidates.append(directory / package["license_file"])
            # Rune's workspace-level license is outside each crate directory.
            if not package.get("source"):
                candidates.extend(p for parent in [directory.parent, directory.parent.parent]
                                  for p in parent.glob("LICENSE*"))
                candidates.extend(directory.glob("assets/*LICENSE*"))
            for path in set(candidates):
                if path.is_file():
                    shutil.copy(path, destination / path.name)
                elif path.is_dir():
                    shutil.copytree(path, destination / path.name, dirs_exist_ok=True)
            # Registry .crate files preserve the exact published sources/notices.
            if (package.get("source") or "").startswith("registry+"):
                archive = directory.parents[2] / "cache" / directory.parent.name / f"{directory.name}.crate"
                if not archive.is_file():
                    raise RuntimeError(f"Missing dependency source archive: {archive}")
                sources.write(archive, f"{name}.crate")
            else:
                for path in sorted(directory.rglob("*")):
                    if path.is_file() and not path.is_symlink():
                        sources.write(path, f"{name}/{path.relative_to(directory)}")
    (notices / "INDEX.txt").write_text("\n".join(index), encoding="utf-8")
    shutil.copy(ROOT / "Cargo.lock", notices / "Cargo.lock")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "artifacts/releases")
    parser.add_argument("--sign-identity", default=os.environ.get("GRIZ_SIGN_IDENTITY", "-"),
                        help="Developer ID identity; default is ad-hoc (-), not Gatekeeper trusted")
    parser.add_argument("--notary-profile", default=os.environ.get("GRIZ_NOTARY_PROFILE"),
                        help="Optional notarytool keychain profile; requires Developer ID signing")
    args = parser.parse_args()
    if platform.system() != "Darwin" or platform.machine() != "arm64":
        parser.error("Run this packager on an Apple Silicon Mac")
    if args.notary_profile and args.sign_identity == "-":
        parser.error("Notarization requires --sign-identity with a Developer ID certificate")
    for tool in ["cargo", "git", "lipo", "otool", "vtool", "codesign", "ditto", "plutil", "xcrun"]:
        if not shutil.which(tool):
            parser.error(f"Required build tool not found: {tool}")
    if args.notary_profile and not shutil.which("spctl"):
        parser.error("Required notarization verification tool not found: spctl")

    args.output = args.output.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    if not (ROOT / "vendor/ghostty-vt-1.0.0.xcframework/ghostty-vt.xcframework/macos-arm64_x86_64/libghostty-vt.a").is_file():
        run("sh", "scripts/setup-ghostty.sh")
    metadata = json.loads(output("cargo", "metadata", "--locked", "--format-version", "1", "--filter-platform", TARGET))
    package = next(p for p in metadata["packages"] if Path(p["manifest_path"]) == ROOT / "Cargo.toml")
    version = package["version"]
    commit, dirty = revision(ROOT)
    label = f"{version}-{commit[:8]}{'-dirty' if dirty else ''}"
    name = f"Griz-{label}-macos-arm64"
    environment = os.environ.copy()
    environment.setdefault("MACOSX_DEPLOYMENT_TARGET", "13.0")
    run("cargo", "build", "--release", "--locked", "--target", TARGET,
        "-p", "griz", "-p", "griz-core", env=environment)
    binaries = Path(metadata["target_directory"]) / TARGET / "release"

    with tempfile.TemporaryDirectory(prefix=".griz-package-", dir=args.output) as temporary:
        stage = Path(temporary)
        payload = stage / name
        app = payload / "Griz.app"
        macos = app / "Contents/MacOS"
        resources = app / "Contents/Resources"
        macos.mkdir(parents=True)
        resources.mkdir()
        linkage = []
        for binary in ["griz", "griz-core"]:
            linkage.append(check_binary(binaries / binary))
            linkage.append(output("vtool", "-show-build", str(binaries / binary)))
            shutil.copy2(binaries / binary, macos / binary)
            (macos / binary).chmod(0o755)
        if not (ROOT / "assets/app-icon/Griz.icns").is_file():
            run("sh", "scripts/build-icon.sh")
        shutil.copy(ROOT / "assets/app-icon/Griz.icns", resources / "Griz.icns")
        (resources / "Griz.icns").chmod(0o644)
        info = {
            "CFBundleDevelopmentRegion": "en", "CFBundleDisplayName": "Griz",
            "CFBundleName": "Griz", "CFBundleIdentifier": BUNDLE_ID,
            "CFBundleExecutable": "griz", "CFBundlePackageType": "APPL",
            "CFBundleIconFile": "Griz", "CFBundleShortVersionString": version,
            "CFBundleVersion": output("git", "rev-list", "--count", "HEAD"),
            "CFBundleGetInfoString": f"Griz {label} (Apple Silicon early test)",
            "GrizBuildRevision": commit, "GrizBuildDirty": dirty,
            "LSMinimumSystemVersion": environment["MACOSX_DEPLOYMENT_TARGET"],
            "NSHighResolutionCapable": True,
        }
        with (app / "Contents/Info.plist").open("wb") as stream:
            plistlib.dump(info, stream)
        run("plutil", "-lint", str(app / "Contents/Info.plist"))
        rune = Path(next(p["manifest_path"] for p in metadata["packages"] if p["name"] == "rune")).parent.parent.parent
        rune_commit, rune_dirty = revision(rune)
        details = [f"Griz {label}", f"Commit: {commit}", f"Working tree modified: {dirty}",
                   f"Rune: {rune_commit} (modified: {rune_dirty})", f"Target: {TARGET}",
                   f"Built: {datetime.datetime.now(datetime.timezone.utc).isoformat()}",
                   f"Build host macOS: {platform.mac_ver()[0]}",
                   f"Deployment target (not a compatibility test): {environment['MACOSX_DEPLOYMENT_TARGET']}",
                   output("rustc", "--version"), f"Signing: {args.sign_identity}",
                   f"Notarization requested: {bool(args.notary_profile)}", "", *linkage]
        (resources / "BUILD-INFO.txt").write_text("\n".join(details) + "\n", encoding="utf-8")
        shutil.copy(resources / "BUILD-INFO.txt", payload / "BUILD-INFO.txt")
        shutil.copy(ROOT / "docs/EARLY-TESTING.md", payload / "READ-ME-FIRST.md")
        dependency_materials(metadata, resources)
        signed_options = [] if args.sign_identity == "-" else ["--timestamp", "--options", "runtime"]
        # Sign inside-out; do not use --deep to paper over unsigned nested code.
        for path in [macos / "griz-core", macos / "griz", app]:
            run("codesign", "--force", "--sign", args.sign_identity, *signed_options, str(path))
        run("codesign", "--verify", "--deep", "--strict", "--verbose=2", str(app))
        if args.notary_profile:
            submission = stage / "notarize.zip"
            run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(app), str(submission))
            run("xcrun", "notarytool", "submit", str(submission), "--keychain-profile", args.notary_profile, "--wait")
            run("xcrun", "stapler", "staple", str(app))
            run("xcrun", "stapler", "validate", str(app))
            run("spctl", "--assess", "--type", "execute", "--verbose=2", str(app))
        archive = args.output / f"{name}.zip"
        run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", str(payload), str(archive))
        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        archive.with_suffix(".zip.sha256").write_text(f"{digest}  {archive.name}\n", encoding="utf-8")
        print(f"\nTester zip: {archive}\nSHA-256:    {digest}")
        if args.sign_identity == "-":
            print("Ad-hoc signed, NOT notarized: testers may need Privacy & Security → Open Anyway.")
        print("No running Griz cores or terminal sessions were restarted.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, RuntimeError, subprocess.CalledProcessError) as error:
        print(f"package-macos: {error}", file=sys.stderr)
        sys.exit(1)
