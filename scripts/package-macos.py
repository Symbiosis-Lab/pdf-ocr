#!/usr/bin/env python3
"""Package the release for macOS: moss-docs, libpdfium, and a relocated Tesseract
with its libraries and English language data, as one tar.gz a moss plugin can
download. Needs Homebrew's tesseract (its build and data are what is shipped)
and a release build (scripts/build.sh).

    python3 -I scripts/package-macos.py            # writes dist/moss-docs-<version>-darwin-arm64.tar.gz

The bundle is self-contained: every library Tesseract loads from Homebrew is
copied beside it and its load commands rewritten to @executable_path/@loader_path,
then everything is re-signed ad hoc (rewriting a Mach-O invalidates its signature).
Archives carry no symlinks, because moss refuses them when it unpacks a stack.
"""
import hashlib, os, pathlib, platform, re, shutil, subprocess, sys, tarfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
BREW = pathlib.Path('/opt/homebrew')

def run(*a):
    return subprocess.run(a, check=True, capture_output=True, text=True).stdout

def deps(f):
    return [l.strip().split(' (')[0] for l in run('otool', '-L', str(f)).splitlines()[1:]]

def local(d):
    return d.startswith(str(BREW)) or d.startswith('@rpath/')

def main():
    if platform.machine() != 'arm64':
        sys.exit('build on Apple silicon: the bundle is darwin-arm64')
    version = re.search(r'^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M).group(1)
    exe = ROOT / 'target/release/moss-docs'
    if not exe.is_file() or not (ROOT / 'target/release/libpdfium.dylib').is_file():
        sys.exit('run scripts/build.sh first')
    tess = pathlib.Path(os.path.realpath(BREW / 'bin/tesseract'))

    out = ROOT / 'dist' / f'moss-docs-{version}-darwin-arm64'
    shutil.rmtree(out, ignore_errors=True)
    (out / 'lib').mkdir(parents=True)
    (out / 'tessdata').mkdir()
    (out / 'licenses').mkdir()
    shutil.copy(exe, out / 'moss-docs')
    shutil.copy(ROOT / 'target/release/libpdfium.dylib', out / 'libpdfium.dylib')
    shutil.copy(tess, out / 'tesseract')
    shutil.copy(os.path.realpath(BREW / 'share/tessdata/eng.traineddata'), out / 'tessdata/eng.traineddata')

    # every Homebrew library reachable from tesseract, keyed by the name it is loaded as
    libdirs = {pathlib.Path(os.path.realpath(p)) for p in BREW.glob('opt/*/lib')}
    def resolve(d):
        if d.startswith('@rpath/'):
            name = d.split('/', 1)[1]
            for ld in libdirs:
                if (ld / name).exists():
                    return pathlib.Path(os.path.realpath(ld / name))
            sys.exit(f'cannot resolve {d}')
        return pathlib.Path(os.path.realpath(d))
    libs, todo = {}, [tess]
    while todo:
        f = todo.pop()
        for d in deps(f):
            if local(d):
                name = os.path.basename(d)
                if name not in libs:
                    libs[name] = resolve(d)
                    todo.append(libs[name])
    for name, src in libs.items():
        shutil.copy(src, out / 'lib' / name)
        os.chmod(out / 'lib' / name, 0o644)

    # rewrite load commands: the executable finds lib/, each library finds its siblings
    def relink(f, prefix):
        for d in deps(f):
            if local(d):
                run('install_name_tool', '-change', d, f'{prefix}/{os.path.basename(d)}', str(f))
    relink(out / 'tesseract', '@executable_path/lib')
    for name in libs:
        f = out / 'lib' / name
        run('install_name_tool', '-id', f'@loader_path/{name}', str(f))
        relink(f, '@loader_path')
    for f in [out / 'tesseract', *sorted((out / 'lib').iterdir())]:
        run('codesign', '--force', '--sign', '-', str(f))
        left = [d for d in deps(f) if local(d) or d.startswith(str(BREW))]
        if left:
            sys.exit(f'{f.name} still loads {left}')

    # licences of what ships: pdfium's from its archive, Homebrew's for the rest
    shutil.copy(ROOT / 'vendor/LICENSE', out / 'licenses' / 'pdfium-LICENSE')
    if (ROOT / 'vendor/licenses').is_dir():
        shutil.copytree(ROOT / 'vendor/licenses', out / 'licenses' / 'pdfium-third-party')
    for formula in sorted({p.parts[len(BREW.parts) + 1] for p in [tess, *libs.values()]}):
        for f in (BREW / 'Cellar' / formula).glob('*/*'):
            if re.match(r'(LICEN[CS]E|COPYING|NOTICE)', f.name, re.I) and f.is_file():
                shutil.copy(f, out / 'licenses' / f'{formula}-{f.name}')
    shutil.copy(ROOT / 'LICENSE', out / 'licenses' / 'moss-docs-LICENSE')
    # licences Homebrew does not keep (leptonica), copied once from upstream at the shipped version
    for f in (ROOT / 'licenses').iterdir():
        shutil.copy(f, out / 'licenses' / f.name)

    tgz = out.parent / f'{out.name}.tar.gz'
    def anonymous(info):
        info.uid = info.gid = 0
        info.uname = info.gname = ''
        return info
    with tarfile.open(tgz, 'w:gz') as t:
        t.add(out, arcname='moss-docs', filter=anonymous)
    digest = hashlib.sha256(tgz.read_bytes()).hexdigest()
    print(f'{tgz}\nsha256 {digest}\n{len(libs)} libraries, tesseract {run(str(out / "tesseract"), "--version").splitlines()[0]}')

if __name__ == '__main__':
    main()
