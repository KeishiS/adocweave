import { SCOPES, finishResponse, diagnostic, emptyResponse, validateRequest } from "./protocol.mjs";
import { loadNotices } from "./notices.mjs";

export async function processRequest(input) {
  const response = emptyResponse();
  let request;
  try {
    request = validateRequest(input);
  } catch (error) {
    response.diagnostics.push(diagnostic(null, null, error.code ?? "invalid-request", error.message));
    return finishResponse(response);
  }
  response.notices = loadNotices(
    SCOPES.some((scope) => request.scopes[scope].equations.length > 0),
    SCOPES.some((scope) => request.scopes[scope].citations.length > 0),
  );
  for (const scope of SCOPES) {
    const { equations, citations } = request.scopes[scope];
    if (equations.length) {
      try {
        const { renderEquations } = await import("./math.mjs");
        const result = await renderEquations(scope, equations, request.eqnums, request.macros ?? [], request.extensions);
        response.scopes[scope].equations = result.results;
        response.diagnostics.push(...result.diagnostics);
      } catch (error) {
        for (const { key } of equations) response.diagnostics.push(diagnostic(scope, key, "math-processing-error", String(error.message ?? error)));
      }
    }
    if (citations.length) {
      const { renderCitations } = await import("./citations.mjs");
      const result = renderCitations(scope, citations, request.csl);
      response.scopes[scope].citations = result.results
        .filter(({ status }) => status === "ok")
        .map(({ key, inlines }) => ({ key, inlines }));
      response.scopes[scope].bibliography = result.bibliography;
      response.diagnostics.push(...result.diagnostics);
    }
  }
  return finishResponse(response);
}
