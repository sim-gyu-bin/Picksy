"""기존 합성 24건을 그대로 보내고 실제 CPU HTTP 지연·품질을 관측한다."""
import argparse
import hashlib
import http.client
import json
import math
import os
from pathlib import Path
import secrets
import statistics
import subprocess
import time
import uuid

ROOT = Path(__file__).resolve().parents[2]
# 채택한 Q4 파일과 해시만 허용해 다른 체크포인트가 섞이는 것을 막는다.
MODEL_FILE = 'd1-2B-Q4_K_M.gguf'
MODEL_SHA = 'adca5789ad30ae6027e8e936749bb9f19673097492ab26fa1c5b69643ca50b4b'


def command(*args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, text=True, **kwargs).stdout.strip()


def assess(answer, question, expected):
    kind = question['type']
    if kind == 'noul':
        p = answer['noul']
        assert math.isfinite(p) and 0 <= p <= 1
        return p > .5 if expected else p < .5
    probs = answer['probabilities']
    assert all(math.isfinite(p) and 0 <= p <= 1 for p in probs.values())
    assert abs(sum(probs.values()) - 1) <= .000151
    if kind == 'choice':
        assert set(probs) == set(question['criteria']) and answer['choice'] in probs
        return answer['choice'] == expected
    assert set(probs) == {'0', '1', '2'}
    assert math.isfinite(answer['score'])
    # llama.cpp 5d4c8eeb2의 lfm3-d1은 평균이 아니라 누적확률 0.5의 첫 등급을 반환한다.
    # 정답 허용 범위는 그대로 두고 서버 고유 계약만 검사한다. 응답값을 변환하지 않는다.
    mass = 0.0
    median = 2
    for level in range(3):
        mass += probs[str(level)]
        if mass >= .5:
            median = level
            break
    assert answer['score'] == median
    assert answer['legend'] == {str(i): label for i, label in enumerate(question['criteria'])}
    return abs(answer['score'] - expected) <= .5


def replay(path):
    # 기존 실패 보고서는 보존한다. 이미 관측한 body만 재채점하고 지연은 재측정하지 않는다.
    report = json.loads(path.read_text())
    raw = (ROOT / 'experiments/fixtures/short-cases.json').read_bytes()
    assert report['completed'] and report['fixture_sha256'] == hashlib.sha256(raw).hexdigest()
    assert report['model_sha256'] == MODEL_SHA
    for row in report['cases']:
        assert row['status'] == 200
        row['correct'] = assess(row['body']['answers']['result'], row['question'], row['expected'])
        row.pop('contract_error', None)
    for group, summary in report['summary'].items():
        rows = [row for row in report['cases'] if row['group'] == group]
        summary['correct'] = sum(row['correct'] for row in rows)
        summary['contract_errors'] = 0
    report['assessment_contract'] = 'lfm3-d1 median score; original thresholds; saved responses only'
    report['source_report'] = path.name
    destination = path.with_name('report-d1-contract.json')
    with destination.open('x') as stream:
        json.dump(report, stream, ensure_ascii=False, indent=2)
    print(json.dumps({'output': str(destination), 'summary': report['summary']}, ensure_ascii=False, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--image', help='확인한 llama.cpp 이미지 digest 또는 ID')
    parser.add_argument('--replay', type=Path, help='실제 서버 호출 없이 저장 응답의 계약을 재검사')
    args = parser.parse_args()
    if args.replay:
        replay(args.replay)
        return
    if not args.image:
        parser.error('--image 또는 --replay 필요')
    model_dir = ROOT / '.local/picksy-dev/d1-q4'
    model = model_dir / MODEL_FILE
    with model.open('rb') as stream:
        assert hashlib.file_digest(stream, 'sha256').hexdigest() == MODEL_SHA
    raw = (ROOT / 'experiments/fixtures/short-cases.json').read_bytes()
    fixture = json.loads(raw)
    run_id = uuid.uuid4().hex[:10]
    output = model_dir / ('run-' + run_id)
    output.mkdir()
    name = 'picksy-d1-q4-' + run_id
    key = secrets.token_urlsafe(32)
    env = {**os.environ, 'LLAMA_API_KEY': key}
    # 외부 공개 없이 loopback 임의 포트만 사용하고 모델 서버 자체 인증도 유지한다.
    server_args = ['-m', '/models/' + MODEL_FILE, '-ngl', '0', '-t', '4', '-tb', '4',
                   '-c', '2048', '-b', '2048', '-ub', '2048', '-np', '1',
                   '--host', '0.0.0.0', '--port', '8080', '--no-webui', '--no-mmproj',
                   '--log-verbosity', '2']
    report = {'completed': False, 'synthetic_only': True, 'container': name,
              'fixture_sha256': hashlib.sha256(raw).hexdigest(), 'image': args.image,
              'quantization': 'q4', 'model_sha256': MODEL_SHA,
              'server_args': server_args, 'cases': [],
              'summary': {}, 'timing': '호스트 HTTP 왕복, 준비 후 첫 요청 포함, 재시도/추가 워밍업 없음'}
    started = time.perf_counter()
    command('docker', 'run', '-d', '--name', name, '--platform', 'linux/amd64',
            '--cpus', '4', '--memory', '6g', '--memory-swap', '6g', '--read-only',
            '--tmpfs', '/tmp:rw,size=64m', '--cap-drop', 'ALL', '--security-opt', 'no-new-privileges',
            '-p', '127.0.0.1::8080', '-e', 'LLAMA_API_KEY',
            '-v', f'{model_dir}:/models:ro', args.image, *server_args, env=env)
    try:
        port = int(json.loads(command('docker', 'inspect', '--format', '{{json .NetworkSettings.Ports}}', name))['8080/tcp'][0]['HostPort'])
        version = subprocess.run(['docker', 'exec', name, '/app/llama-server', '--version'],
                                 check=True, capture_output=True, text=True)
        report['version'] = (version.stdout + version.stderr).strip()
        def request(path, payload=None, authenticated=True):
            connection = http.client.HTTPConnection('127.0.0.1', port, timeout=120)
            headers = {'Content-Type': 'application/json'}
            if authenticated:
                headers['Authorization'] = 'Bearer ' + key
            try:
                connection.request('GET' if payload is None else 'POST', path,
                                   body=None if payload is None else json.dumps(payload, ensure_ascii=False).encode(), headers=headers)
                response = connection.getresponse()
                return response.status, response.read()
            finally:
                connection.close()
        deadline = time.monotonic() + 180
        while True:
            try:
                status, _ = request('/health')
                if status == 200:
                    break
            except OSError:
                pass
            state = json.loads(command('docker', 'inspect', '--format', '{{json .State}}', name))
            assert state['Running'], '모델 서버 시작 실패'
            assert time.monotonic() < deadline, '준비 시간 초과'
            time.sleep(.25)
        report['ready_seconds'] = time.perf_counter() - started
        assert request('/v1/models', authenticated=False)[0] == 401, '서버 인증 실패'
        for group, spec in fixture['groups'].items():
            question = spec['question']
            for case in spec['cases']:
                # 기대값은 모델에 보내지 않는다. 순서·문구·판정 범위는 기존 Q4와 동일하다.
                begin = time.perf_counter()
                status, body = request('/v1/systemone', {'state': case['state'], 'questions': {'result': question}})
                row = {'group': group, **case, 'question': question, 'status': status,
                       'elapsed_ms': (time.perf_counter() - begin) * 1000, 'correct': False}
                try:
                    row['body'] = json.loads(body)
                    assert status == 200
                    row['correct'] = assess(row['body']['answers']['result'], question, case['expected'])
                except (AssertionError, ValueError, KeyError, TypeError) as error:
                    row['contract_error'] = type(error).__name__
                report['cases'].append(row)
        for group in fixture['groups']:
            selected = [r for r in report['cases'] if r['group'] == group]
            times = [r['elapsed_ms'] for r in selected]
            report['summary'][group] = {'correct': sum(r['correct'] for r in selected), 'total': len(selected),
                'contract_errors': sum('contract_error' in r for r in selected),
                'mean_ms': statistics.mean(times), 'median_ms': statistics.median(times),
                'min_ms': min(times), 'max_ms': max(times)}
        report['process_memory'] = '\n'.join(line for line in command('docker', 'exec', name, 'cat', '/proc/1/status').splitlines()
                                             if line.startswith(('VmRSS:', 'VmHWM:')))
        report['completed'] = True
    finally:
        logs = subprocess.run(['docker', 'logs', name], capture_output=True, text=True, check=True)
        # 비밀값이 로그에 있으면 원문을 저장하지 않고 실패한다.
        safe_logs = key not in logs.stdout + logs.stderr
        report['key_absent_from_logs'] = safe_logs
        if safe_logs:
            (output / 'server.log').write_text(logs.stdout + logs.stderr)
        command('docker', 'stop', '--time', '30', name)
        report['final_state'] = json.loads(command('docker', 'inspect', '--format', '{{json .State}}', name))
        with (output / 'report.json').open('x') as stream:
            json.dump(report, stream, ensure_ascii=False, indent=2)
        print(json.dumps({'output': str(output.relative_to(ROOT)), 'completed': report['completed'],
                          'summary': report['summary'], 'memory': report.get('process_memory'),
                          'final_state': report['final_state'], 'key_absent_from_logs': safe_logs}, ensure_ascii=False, indent=2))
        assert safe_logs, '서버 로그의 키 노출 감지'
    assert all('contract_error' not in r for r in report['cases']), '응답 계약 오류'


if __name__ == '__main__':
    main()
