import hashlib
import json
import os
import pathlib
import subprocess
import sys
import tempfile

os.umask(0o077)

config_dir = pathlib.Path(sys.argv[1])
source_zones_dir = pathlib.Path(sys.argv[2])
runtime_zones_dir = pathlib.Path(sys.argv[3])
requested_zone_files = {
    f"{zone[:-1] if zone.endswith('.') else zone}.yaml"
    for zone in sys.argv[4:]
    if zone and not zone.startswith("-")
}
source_zone_paths = {
    str(source_zones_dir),
    str(source_zones_dir.resolve()),
}
replacements = json.loads(pathlib.Path(sys.argv[5]).read_text())
secret_dir = os.environ.get("CANIX_DNS_SECRET_DIR")
decrypt_cache_dir = os.environ.get("CANIX_DNS_DECRYPT_CACHE_DIR")
age_identities = [
    identity
    for identity in os.environ.get("CANIX_DNS_AGE_IDENTITIES", "").split(":")
    if identity
]


def ensure_cache_dir():
    if decrypt_cache_dir is None:
        return None
    cache_dir = pathlib.Path(decrypt_cache_dir)
    cache_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    os.chmod(cache_dir, 0o700)
    mode = cache_dir.stat().st_mode & 0o777
    if mode != 0o700:
        raise RuntimeError(f"cache dir perms not 0700: {cache_dir}")
    return cache_dir


def cache_key_for(path):
    digest = hashlib.sha256()
    with path.open("rb") as encrypted:
        for chunk in iter(lambda: encrypted.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def read_cached_secret(path):
    cache_dir = ensure_cache_dir()
    if cache_dir is None:
        return None
    cache_file = cache_dir / cache_key_for(path)
    if not cache_file.exists():
        return None
    mode = cache_file.stat().st_mode & 0o777
    if mode != 0o600:
        raise RuntimeError(f"cache file perms not 0600: {cache_file}")
    return cache_file.read_text()


def write_cached_secret(path, plaintext):
    cache_dir = ensure_cache_dir()
    if cache_dir is None:
        return
    cache_file = cache_dir / cache_key_for(path)
    fd, tmp_name = tempfile.mkstemp(prefix=".tmp.", dir=str(cache_dir))
    tmp_path = pathlib.Path(tmp_name)
    try:
        os.fchmod(fd, 0o600)
        os.write(fd, plaintext.encode())
        os.fsync(fd)
        os.close(fd)
        fd = None
        os.replace(tmp_path, cache_file)
        os.chmod(cache_file, 0o600)
        mode = cache_file.stat().st_mode & 0o777
        if mode != 0o600:
            raise RuntimeError(f"cache file perms not 0600: {cache_file}")
    except Exception:
        if fd is not None:
            os.close(fd)
        try:
            tmp_path.unlink()
        except FileNotFoundError:
            pass
        raise


def read_secret(replacement):
    runtime_path = replacement.get("path")
    if runtime_path is not None:
        path = pathlib.Path(runtime_path)
        if path.exists():
            return path.read_text().strip()
        if secret_dir is not None:
            path = pathlib.Path(secret_dir) / pathlib.Path(runtime_path).name
            if path.exists():
                return path.read_text().strip()

    agenix_file = replacement.get("agenixFile")
    if agenix_file is not None:
        path = pathlib.Path(agenix_file)
        if not path.exists():
            print(f"missing DNS agenix source file: {path}", file=sys.stderr)
            sys.exit(1)
        cached_secret = read_cached_secret(path)
        if cached_secret is not None:
            return cached_secret.strip()
        secret_manager_bin = os.environ.get("SECRET_MANAGER_BIN")
        if not secret_manager_bin:
            print("SECRET_MANAGER_BIN is not set", file=sys.stderr)
            sys.exit(1)
        readable_identities = [
            identity
            for identity in age_identities
            if pathlib.Path(identity).is_file() and os.access(identity, os.R_OK)
        ]
        if readable_identities:
            cmd = [secret_manager_bin, "decrypt"]
            for identity in readable_identities:
                cmd.extend(["--identity", identity])
            cmd.append(str(path))
            result = subprocess.run(cmd, check=False, text=True, capture_output=True)
            if result.returncode == 0:
                write_cached_secret(path, result.stdout)
                return result.stdout.strip()
            print(result.stderr, file=sys.stderr, end="")
            print(f"failed to decrypt DNS agenix source file: {path}", file=sys.stderr)
            sys.exit(result.returncode)

    missing = runtime_path or agenix_file
    print(f"missing DNS secret file: {missing}", file=sys.stderr)
    if agenix_file is not None and not age_identities:
        print(
            "set CANIX_DNS_AGE_IDENTITIES to colon-separated age/ssh identity paths to decrypt the agenix source locally",
            file=sys.stderr,
        )
    sys.exit(1)


for path in config_dir.rglob("*.yaml"):
    text = path.read_text()
    for source_zone_path in source_zone_paths:
        text = text.replace(source_zone_path, str(runtime_zones_dir))
    should_substitute_secrets = (
        not requested_zone_files
        or path.parent.name != "zones"
        or path.name in requested_zone_files
    )
    if not should_substitute_secrets:
        path.write_text(text)
        continue
    for replacement in replacements:
        if replacement["placeholder"] not in text:
            continue
        value = json.dumps(read_secret(replacement))[1:-1]
        text = text.replace(replacement["placeholder"], value)
    path.write_text(text)
