import argparse
import hashlib
import json
import platform
import subprocess
from datetime import datetime, timezone
from pathlib import Path


def reject_constant(value):
    raise ValueError('nonfinite JSON value: ' + value)


def capture(binary: Path, output: Path, samples: int, *, timeout: float = 1800) -> dict:
    if output.exists():
        raise FileExistsError(output)
    binary = binary.resolve(strict=True)
    command = [str(binary), '--samples', str(samples), '--output', str(output.resolve())]
    with output.with_suffix('.log').open('x') as log:
        result = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, timeout=timeout)
    if result.returncode:
        raise RuntimeError(str(binary) + ' exited ' + str(result.returncode))
    report = json.loads(output.read_text(), parse_constant=reject_constant)
    json.dumps(report, allow_nan=False)
    if (not isinstance(report, dict)
            or report.get('sample_count') != samples
            or not isinstance(report.get('metrics'), dict)
            or not report['metrics']):
        raise ValueError('missing metrics or incompatible sample count')
    with binary.open('rb') as binary_file:
        digest = hashlib.file_digest(binary_file, 'sha256').hexdigest()
    return {'binary': str(binary), 'sha256': digest,
            'command': command, 'report': output.name,
            'captured_at': datetime.now(timezone.utc).isoformat()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--bin-dir', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--samples', type=int, default=30)
    args = parser.parse_args()
    if not 30 <= args.samples <= 100000:
        parser.error('--samples must be between 30 and 100000')
    repo = Path(__file__).resolve().parents[1]
    def git(*arguments):
        return subprocess.check_output(['git', '-C', str(repo), *arguments], text=True).strip()
    if git('status', '--porcelain'):
        parser.error('commit the measured checkout before capture')
    args.output.mkdir(parents=True, exist_ok=False)
    manifest = {'schema': 1, 'source_commit': git('rev-parse', 'HEAD'),
                'platform': platform.platform(), 'machine': platform.machine(),
                'python': platform.python_version(), 'samples': args.samples,
                'profile': 'caller-built; see recorded release build commands', 'runs': [], 'status': 'incomplete',
                'unmeasured': ['native GPU presentation', 'live IPC round trip',
                               'eight live pane CPU/RAM', 'energy', 'model-backed task time/tokens']}
    try:
        for name in ('sprite-term-bench', 'sprite-observation-bench', 'sprite-paint-bench'):
            manifest['runs'].append(capture(args.bin_dir / name, args.output / (name + '.json'), args.samples))
        manifest['status'] = 'component-capture-complete'
    except Exception as error:
        manifest['error'] = str(error)
        raise
    finally:
        (args.output / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')


if __name__ == '__main__':
    main()
