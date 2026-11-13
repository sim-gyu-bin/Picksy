export interface Session {
  authenticated: true;
  csrf_token: string;
  expires_at: number;
}
export interface KeyInfo {
  id: string;
  name: string | null;
  display_name: string;
  created_at: number;
  revoked_at: number | null;
  requests_per_minute: number;
}
export interface CreatedKey {
  id: string;
  api_key: string;
  name: string | null;
  display_name: string;
}
export type RequestKind = "inference" | "models" | "auth_check" | "admin" | "unknown";
export interface UsageSummary {
  total: number;
  pending: number;
  succeeded: number;
  rejected: number;
  failed: number;
  other_completed: number;
  input_tokens: number | null;
  output_tokens: number | null;
  token_recorded: number;
  token_unknown: number;
}
// sqlx(flatten)은 DB 행 매핑용이다. HTTP JSON은 summary가 중첩된다.
export interface UsageReport {
  summary: UsageSummary;
  by_day: { day: string; summary: UsageSummary }[];
  by_kind: { kind: RequestKind; summary: UsageSummary }[];
}
