"""고정 짧은 합성 질문을 원본 Q4 설정에서 하나씩 판단한다."""
import hashlib
import json
import math
import statistics
import time
import resource
import importlib.metadata
from pathlib import Path
import torch
from decider.infer import Decider


def main():
    raw = Path(__file__).with_name('short-cases.json').read_bytes()
    fixture = json.loads(raw)
    torch.set_num_threads(4)
    model = Decider('/models/decider/decider-2b-v12-Q4_K_M.gguf', device='cpu',
                    gguf_options={'n_gpu_layers':0,'n_threads':4,'n_ctx':2048,'n_seq_max':1,'verbose':True})
    report = {'synthetic_only':True,'fixture_sha256':hashlib.sha256(raw).hexdigest(),
              'packages':{n:importlib.metadata.version(n) for n in ('decider-ai','llama-cpp-python')},
              'temperature_by_type':model.T_by_type,'isolated_levels':model.isolated_levels,
              'cases':[],'summary':{},'completed':False}
    rows = []
    original_decode = model.eng._decode
    def decode(items):
        rows.extend(len(ids) for ids, _ in items)
        return original_decode(items)
    model.eng._decode = decode
    try:
        for group, spec in fixture['groups'].items():
            q = spec['question']; kind = q['type']
            for case in spec['cases']:
                rows.clear()
                started = time.perf_counter()
                response = model.system_one(case['state'], {'result':q}, max_state_tokens=2048)
                elapsed = (time.perf_counter()-started)*1000
                a = response['answers']['result']
                if kind == 'noul':
                    p = a['noul']; assert math.isfinite(p) and 0 <= p <= 1
                    # 0.5는 어느 쪽에도 확신이 없으므로 정답으로 세지 않는다.
                    correct = p > .5 if case['expected'] else p < .5
                else:
                    probs = a['probabilities']
                    assert all(math.isfinite(p) and 0 <= p <= 1 for p in probs.values())
                    assert abs(sum(probs.values())-1) <= .000151
                    if kind == 'choice':
                        assert set(probs)==set(q['criteria']) and a['choice'] in probs
                        correct = a['choice']==case['expected']
                    else:
                        assert set(probs)=={'0','1','2'}
                        assert math.isfinite(a['score']) and abs(a['score']-sum(int(k)*p for k,p in probs.items())) <= .0052
                        correct = abs(a['score']-case['expected']) <= .5
                report['cases'].append({'group':group,**case,'question':q,'body':response,
                                        'correct':correct,'elapsed_ms':elapsed,'row_tokens':list(rows)})
        for group in fixture['groups']:
            selected = [r for r in report['cases'] if r['group']==group]
            report['summary'][group] = {'correct':sum(r['correct'] for r in selected),'total':len(selected),
                'mean_ms':statistics.mean(r['elapsed_ms'] for r in selected),
                'median_ms':statistics.median(r['elapsed_ms'] for r in selected),
                'min_ms':min(r['elapsed_ms'] for r in selected),'max_ms':max(r['elapsed_ms'] for r in selected)}
        report['completed'] = True
    finally:
        report['peak_rss_kib'] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
        print(json.dumps(report,ensure_ascii=False,indent=2),flush=True)


if __name__ == '__main__':
    main()
