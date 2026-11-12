"""가중치·판정 설정을 바꾸지 않고 대표 요청의 실행 구간을 계측한다."""
import json
import time
import resource
from pathlib import Path
import torch
import llama_cpp
from decider.infer import Decider


def cpu_stat():
    return {k: int(v) for k, v in (line.split() for line in Path('/sys/fs/cgroup/cpu.stat').read_text().splitlines())}


def main():
    torch.set_num_threads(4)
    fixture = json.loads(Path('/experiment-stage1/score-cases.json').read_text())
    case = fixture['cases'][0]
    domain = fixture['domains'][case['domain']]
    levels = domain['criteria']['ko']
    questions = {'score': {'type': 'score', 'instructions': domain['instructions']['ko'], 'criteria': levels},
                 'choice': {'type': 'choice', 'instructions': domain['instructions']['ko'], 'criteria': dict(zip(('A','B','C'), levels))}}
    model = Decider('/models/decider/decider-2b-v12-Q8_0.gguf', device='cpu',
                    gguf_options={'n_gpu_layers':0,'n_threads':4,'n_ctx':2048,'n_seq_max':1,'verbose':True})
    report = {'system': llama_cpp.llama_print_system_info().decode(), 'cpu_max': Path('/sys/fs/cgroup/cpu.max').read_text().strip(),
              'case_id':case['id'], 'runs':[]}
    active = None
    # 래퍼는 동일 인자·반환값을 유지하며 호출 시간만 누적한다.
    def timed(obj, name):
        original = getattr(obj, name)
        def wrapped(*args, **kwargs):
            start = time.perf_counter()
            try:
                return original(*args, **kwargs)
            finally:
                if active is not None:
                    active.setdefault(name, []).append((time.perf_counter()-start)*1000)
        setattr(obj, name, wrapped)
    for name in ('llama_memory_clear', 'llama_decode'):
        timed(llama_cpp, name)
    for name in ('_system_one_items', '_system_one_probs'):
        timed(model, name)
    original_decode = model.eng._decode
    def decode(rows):
        active.setdefault('row_tokens', []).append([len(ids) for ids, _ in rows])
        return original_decode(rows)
    model.eng._decode = decode
    for shape in ('original','paired2'):
        state = case['state']['ko'] if shape == 'original' else {'text':case['state']['ko'],'measurement_id':'98f82b99-597a-4d7c-949c-139b2e2c11cd'}
        for kinds in (('score','choice'), ('choice',)):
            active = {'shape':shape,'questions':list(kinds)}
            before = cpu_stat()
            cpu_before = time.process_time()
            start = time.perf_counter()
            response = model.system_one(state, {k:questions[k] for k in kinds}, max_state_tokens=2048)
            active['elapsed_ms'] = (time.perf_counter()-start)*1000
            active['process_cpu_ms'] = (time.process_time()-cpu_before)*1000
            after = cpu_stat()
            active['cgroup_delta'] = {k:after[k]-before[k] for k in before}
            active['response'] = response
            report['runs'].append(active)
            active = None
    report['peak_rss_kib'] = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    print(json.dumps(report,ensure_ascii=False,indent=2))


if __name__ == '__main__':
    main()
