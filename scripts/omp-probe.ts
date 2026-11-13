import { JudgmentApiError, NativeJudge, type JudgmentRequest } from "@oh-my-pi/pi-ai/judgment";
import { fetchTypeSafeModels } from "@oh-my-pi/pi-catalog/discovery/typesafe";

// OMP v18.19.0 공식 serializer/parser를 설치 바이너리의 명시적 -e 확장에서 사용한다.
// 근거: https://github.com/can1357/oh-my-pi/tree/v18.19.0/packages/ai/src/judgment
// 오류 status: packages/ai/src/error/classes.ts의 ProviderHttpError.status.
// 독립 HOME·agent 디렉터리·cwd와 환경 allowlist는 실행자가 준비한다.
const request = {
  state: "합성 검증 사례입니다. 어제 결제가 두 번 되어 중복 금액을 환불받고 싶습니다. 서비스는 정상 이용 중입니다.",
  questions: {
    route: {
      type: "choice",
      instructions: "요청을 담당할 부서를 선택하세요.",
      criteria: { billing: "결제와 환불", technical: "기술 장애" },
    },
    refund: { type: "noul", instructions: "환불을 요청하고 있나요?" },
    urgency: {
      type: "score",
      instructions: "요청의 긴급도를 평가하세요.",
      criteria: ["낮음", "보통", "높음"],
    },
  },
} as const satisfies JudgmentRequest;

class ProbeFailure extends Error {
  constructor(readonly label: string) {
    super(label);
  }
}

function requireCheck(condition: unknown, label: string): asserts condition {
  if (!condition) throw new ProbeFailure(label);
}

function record(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function probability(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value) && value >= 0 && value <= 1;
}

function distribution(value: unknown, keys: string[]): number[] {
  requireCheck(record(value), "probabilities_shape");
  requireCheck(Object.keys(value).length === keys.length, "probabilities_keys");
  const values = keys.map((key) => {
    requireCheck(Object.hasOwn(value, key) && probability(value[key]), "probabilities_range");
    return value[key] as number;
  });
  requireCheck(Math.abs(values.reduce((sum, p) => sum + p, 0) - 1) <= 0.0001, "probabilities_sum");
  return values;
}

type Transport = {
  check: string;
  method: string;
  path: string;
  startedAt: string;
  elapsedMs?: number;
  status?: number;
  aborted?: boolean;
};
type ProbeFetch = NonNullable<ConstructorParameters<typeof NativeJudge>[0]["fetch"]>;

export default async function (): Promise<void> {
  const checks: Record<string, unknown> = {};
  const transport: Transport[] = [];
  const marker = {
    ok: false,
    checks,
    api: "typesafe",
    provider: "picksy",
    model: "d1-2b-q4",
    answerTypes: {} as Record<string, string>,
    inputTokens: null as number | null,
    outputTokens: null as number | null,
  };
  let stage = "configuration";
  try {
    const baseUrl = process.env.PICKSY_PROBE_BASE_URL;
    const apiKey = process.env.PICKSY_PROBE_KEY;
    const model = process.env.PICKSY_PROBE_MODEL ?? "d1-2b-q4";
    const mode = process.env.PICKSY_PROBE_MODE ?? "normal";
    requireCheck(baseUrl && apiKey && model === "d1-2b-q4", "configuration");
    const url = new URL(baseUrl);
    requireCheck(
      (url.protocol === "http:" || url.protocol === "https:") &&
        !url.username && !url.password && !url.search && !url.hash && url.pathname === "/",
      "base_url_root_required",
    );
    requireCheck(mode === "normal" || mode === "cancel", "mode");
    marker.model = model;
    checks.mode = mode;
    checks.runId = crypto.randomUUID();
    checks.startedAt = new Date().toISOString();
    checks.transport = transport;
    let wireResult: unknown;

    // 실제 fetch를 그대로 위임한다. 응답을 교체하지 않고 정상 응답만 메모리에서 비교한다.
    // 시각·시도 수·고정 경로만 공개하여 DB/permit/상류 관측과 대조할 수 있게 한다.
    const observedFetch: ProbeFetch = async (input, init) => {
      const target = new URL(input instanceof Request ? input.url : String(input));
      requireCheck(target.origin === url.origin &&
        (target.pathname === "/v1/models" || target.pathname === "/v1/systemone"), "unexpected_target");
      const event: Transport = {
        check: stage,
        method: init?.method ?? "GET",
        path: target.pathname,
        startedAt: new Date().toISOString(),
      };
      transport.push(event);
      const start = performance.now();
      try {
        // 리다이렉트가 인증키를 다른 경계로 옮기는 것을 허용하지 않는다.
        const response = await fetch(input, { ...init, redirect: "error" });
        event.status = response.status;
        if (stage === "judgment" && response.ok) wireResult = await response.clone().json();
        return response;
      } catch (error) {
        event.aborted = init?.signal?.aborted === true;
        throw error;
      } finally {
        event.elapsedMs = Math.round(performance.now() - start);
      }
    };
    const judge = (key = apiKey, selectedModel = model) => new NativeJudge({
      api: "typesafe", provider: "picksy", baseUrl, model: selectedModel,
      apiKey: key, fetch: observedFetch, timeoutMs: 35_000,
    });

    if (mode === "cancel") {
      stage = "cancel";
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 50);
      let cancelled = false;
      try {
        await judge().judge(request, { signal: controller.signal });
      } catch (error) {
        cancelled = controller.signal.aborted && !(error instanceof JudgmentApiError) &&
          (error === controller.signal.reason || (error instanceof Error && error.name === "AbortError"));
      } finally {
        clearTimeout(timer);
      }
      requireCheck(cancelled && transport.length === 1 && transport[0].aborted === true, "cancel_not_observed");
      checks.cancelled = true;
      checks.abortAfterMs = 50;
      // 클라이언트 취소만 증명한다. 상류 중단·permit 회수·DB 상태는 실행자가 별도로 확인한다.
    } else {
      stage = "discovery";
      const models = await fetchTypeSafeModels({ baseUrl, apiKey, fetch: observedFetch, signal: AbortSignal.timeout(10_000) });
      const selected = models?.find((entry) => entry.id === model);
      requireCheck(selected && selected.api === "typesafe" && selected.provider === "typesafe" &&
        selected.kind === "judge" && selected.input.includes("text"), "discovery");
      checks.discovery = true;

      stage = "judgment";
      const result = await judge().judge(request, { signal: AbortSignal.timeout(40_000) });
      requireCheck(result.api === "typesafe" && result.provider === "picksy" && result.model === model, "result_identity");
      requireCheck(record(wireResult) && wireResult.model === model && record(wireResult.usage), "wire_identity_usage");
      requireCheck(Object.keys(result.answers).length === 3 &&
        Object.keys(request.questions).every((id) => Object.hasOwn(result.answers, id)), "answer_ids");
      const { route, refund, urgency } = result.answers;
      requireCheck(route.type === "choice" && refund.type === "noul" && urgency.type === "score", "answer_types");
      const choice = distribution(route.probabilities, ["billing", "technical"]);
      requireCheck(Object.hasOwn(route.probabilities, route.choice), "choice_id");
      requireCheck(probability(route.confidence) &&
        Math.abs(route.confidence - Math.max(...choice)) <= 0.0001 &&
        Math.abs(route.probabilities[route.choice] - Math.max(...choice)) <= 0.0001, "choice_confidence");
      requireCheck(probability(refund.noul), "noul_range");
      const scores = distribution(urgency.probabilities, ["0", "1", "2"]);
      let cumulative = 0;
      const median = scores.findIndex((p) => (cumulative += p) >= 0.5);
      // d1의 중앙값과 최대확률을 검사하며 OMP 문서의 가중평균으로 변환하지 않는다.
      requireCheck(median >= 0 && urgency.score === median, "score_median");
      requireCheck(probability(urgency.confidence) &&
        Math.abs(urgency.confidence - Math.max(...scores)) <= 0.0001, "score_confidence");
      requireCheck(Number.isSafeInteger(result.usage.input) && result.usage.input > 0 &&
        Number.isSafeInteger(result.usage.output) && result.usage.output === 0 &&
        result.usage.totalTokens === result.usage.input + result.usage.output &&
        wireResult.usage.input_tokens === result.usage.input &&
        wireResult.usage.output_tokens === result.usage.output, "usage_preservation");
      marker.answerTypes = { route: route.type, refund: refund.type, urgency: urgency.type };
      marker.inputTokens = result.usage.input;
      marker.outputTokens = result.usage.output;
      checks.judgment = true;
      checks.answerIds = true;
      checks.probabilities = true;
      checks.median = true;
      checks.confidence = true;
      checks.wireModelAndUsage = true;

      const expectStatus = async (label: string, client: NativeJudge, body: JudgmentRequest, status: number) => {
        stage = label;
        const before = transport.length;
        let matched = false;
        try {
          await client.judge(body, { signal: AbortSignal.timeout(40_000) });
        } catch (error) {
          // message에는 서버 본문이 포함된다. 클래스와 공식 status 필드만 검사한다.
          matched = error instanceof JudgmentApiError && error.status === status;
        }
        requireCheck(matched && transport.length === before + 1 && transport[before].status === status, label);
        checks[label] = true;
      };
      await expectStatus("invalid_key_401", judge(`picksy-probe-invalid-${crypto.randomUUID()}`), request, 401);
      await expectStatus("unknown_model_400", judge(apiKey, `picksy-probe-unknown-${crypto.randomUUID()}`), request, 400);
      // choice의 빈 criteria는 TS로 표현 가능하지만 Gateway가 거절해야 하는 실제 잘못된 입력이다.
      await expectStatus("invalid_criteria_400", judge(), {
        state: request.state,
        questions: { route: { type: "choice", instructions: "담당 부서를 선택하세요.", criteria: {} } },
      }, 400);
    }
    marker.ok = true;
  } catch (error) {
    checks.error = error instanceof ProbeFailure ? error.label : `failed_${stage}`;
    process.exitCode = 1;
  }
  // 성공만 0으로 끝나며 다른 CLI 실패 코드를 덮어쓰지 않는다. 본문·키·오류 원문은 출력하지 않는다.
  checks.finishedAt = new Date().toISOString();
  console.log(`PICKSY_OMP_PROBE ${JSON.stringify(marker)}`);
}
