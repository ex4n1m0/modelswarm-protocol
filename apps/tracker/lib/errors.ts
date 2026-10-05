// Error model per msp-v1 §3.4: machine-readable code, human message,
// retryable flag, and the frozen code -> HTTP status map.

export type ErrorCode =
  | "unauthorized"
  | "unsigned_request"
  | "stale_timestamp"
  | "replayed_nonce"
  | "rate_limited"
  | "payload_too_large"
  | "invalid_body"
  | "unknown_profile"
  | "unknown_installation"
  | "pending"
  | "forbidden"
  | "revoked"
  | "lease_expired"
  | "no_capability"
  | "ineligible";

/** Frozen mapping (msp-v1 §3.4). unknown_profile is 404 on public catalog
 * reads and 400 on request-body validation; call sites pick explicitly. */
export const ERROR_STATUS: Record<ErrorCode, number> = {
  unauthorized: 401,
  unsigned_request: 401,
  stale_timestamp: 400,
  replayed_nonce: 400,
  rate_limited: 429,
  payload_too_large: 413,
  invalid_body: 400,
  unknown_profile: 400,
  unknown_installation: 401,
  pending: 403,
  forbidden: 403,
  revoked: 401,
  lease_expired: 410,
  no_capability: 403,
  ineligible: 403,
};

export interface JsonErrorBody {
  error: { code: ErrorCode; message: string; retryable: boolean };
}

export function errorBody(code: ErrorCode, message: string, retryable = false): JsonErrorBody {
  return { error: { code, message, retryable } };
}

/**
 * Build a JSON error Response. `status` is explicit because two codes are
 * context-sensitive (unknown_profile 400 vs 404 per msp-v1 §3.1/§3.3).
 */
export function jsonError(
  status: number,
  code: ErrorCode,
  message: string,
  retryable = false,
): Response {
  return new Response(JSON.stringify(errorBody(code, message, retryable)), {
    status,
    headers: { "content-type": "application/json" },
  });
}

/** Error Response using the frozen status for `code`. */
export function jsonErrorFor(code: ErrorCode, message: string, retryable = false): Response {
  return jsonError(ERROR_STATUS[code], code, message, retryable);
}

/** Success JSON Response helper. */
export function jsonOk(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

// Structured logging that can never carry request bodies (F3 / ADR-001).
type LogFields = Record<string, string | number | boolean | null | undefined>;
export function logEvent(level: "info" | "warn" | "error", fields: LogFields): void {
  const line = JSON.stringify({ ts: new Date().toISOString(), level, ...fields });
  if (level === "error") console.error(line);
  else console.log(line);
}
