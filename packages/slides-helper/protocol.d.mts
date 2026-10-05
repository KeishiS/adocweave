export type Scope = "body" | "notes";
export type Eqnums = "none" | "ams" | "all";

export type Equation = {
  key: string;
  tex: string;
  display: boolean;
  footnote?: boolean;
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
  schemaVersion: 2;
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

export type EquationResult = { key: string; svg: string };
export type CitationResult = { key: string; inlines: Inline[] };
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
export type Response =
  | { schemaVersion: 2; status: "ok"; notices: Notices; scopes: { body: ScopeOutput; notes: ScopeOutput }; diagnostics: Diagnostic[] }
  | { schemaVersion: 2; status: "failed"; diagnostics: Diagnostic[] };
