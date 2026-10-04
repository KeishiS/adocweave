export type Scope = "body" | "notes";
export type Eqnums = "none" | "ams" | "all";

export type Equation = {
  key: string;
  tex: string;
  display: boolean;
};

export type CitationItem = {
  id: string;
  locator?: string;
  label?: string;
  prefix?: string;
  suffix?: string;
  suppressAuthor?: boolean;
  authorOnly?: boolean;
};

export type Citation = { key: string; items: CitationItem[] };
export type ScopeInput = { equations: Equation[]; citations: Citation[] };
export type Macro = {
  name: string;
  definition: string;
  arguments?: number;
  default?: string;
};
export type CslItem = { id: string; [field: string]: unknown };
export type Request = {
  schemaVersion: 1;
  eqnums: Eqnums;
  scopes: { body: ScopeInput; notes: ScopeInput };
  macros?: Macro[];
  csl?: { items: CslItem[]; style: string; locale: string };
};

export type Inline =
  | { kind: "text"; text: string }
  | {
      kind: "emphasis" | "strong" | "superscript" | "subscript" | "smallcaps"
        | "normal-emphasis" | "normal-strong" | "normal-smallcaps" | "underline";
      children: Inline[];
    }
  | { kind: "link"; href: string; children: Inline[] };

export type EquationResult =
  | { key: string; status: "ok"; svg: string }
  | { key: string; status: "failed" };
export type CitationResult =
  | { key: string; status: "ok"; inlines: Inline[] }
  | { key: string; status: "failed" };
export type BibliographyEntry = { id: string; inlines: Inline[] };
export type ScopeOutput = {
  equations: EquationResult[];
  citations: CitationResult[];
  bibliography: BibliographyEntry[];
};
export type Diagnostic = {
  scope: Scope | null;
  key: string | null;
  severity: "error" | "warning";
  code: string;
  message: string;
};
export type Notices = {
  math: null | {
    fontAttribution: string;
    fontLicense: string;
    lpplLicense: string;
    mathjaxLicense: string;
  };
  citations: null | { attribution: string; license: string };
};
export type Response = {
  // Every input key appears once, in input order. A failed result has a
  // scoped/keyed error; a key with an error never returns status "ok".
  // Request/output-limit failures instead return empty scopes and a global error.
  schemaVersion: 1;
  // Fixed bundled UTF-8 text only; no requested paths or downloadable resources.
  // Nonempty equation/citation input requires its notice; global failure uses null.
  notices: Notices;
  scopes: { body: ScopeOutput; notes: ScopeOutput };
  diagnostics: Diagnostic[];
};
