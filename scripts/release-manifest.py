#!/usr/bin/env python3
"""Write plugin/manifest.json, the manifest a site installs, naming this
version's release archives by URL and sha256. moss downloads the archive for
its platform into ~/.moss/stacks on first use.

    python3 -I scripts/release-manifest.py      # after scripts/package-macos.py
"""
import hashlib, json, pathlib, re

ROOT = pathlib.Path(__file__).resolve().parent.parent
REPO = 'https://github.com/Symbiosis-Lab/pdf-ocr'
version = re.search(r'^version = "([^"]+)"', (ROOT / 'Cargo.toml').read_text(), re.M).group(1)
artifacts = []
for tgz in sorted((ROOT / 'dist').glob(f'pdf-ocr-{version}-*.tar.gz')):
    platform = tgz.name.removeprefix(f'pdf-ocr-{version}-').removesuffix('.tar.gz')
    artifacts.append({'kind': 'download', 'platform': platform,
                      'url': f'{REPO}/releases/download/v{version}/{tgz.name}',
                      'sha256': hashlib.sha256(tgz.read_bytes()).hexdigest(),
                      'archive_format': 'tar.gz', 'executable': 'pdf-ocr/pdf-ocr'})
if not artifacts:
    raise SystemExit(f'no dist/pdf-ocr-{version}-*.tar.gz: run scripts/package-macos.py')
template = (ROOT / 'plugin/manifest.template.json').read_text()
manifest = json.loads(template.replace('ARTIFACT', '{}'))
manifest['version'] = version
manifest['contributes']['derivations']['pdf-pages']['run']['artifact'] = artifacts
(ROOT / 'plugin/manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')
print('plugin/manifest.json:', ', '.join(a['platform'] for a in artifacts))
