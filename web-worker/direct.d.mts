import type { AdocWeaveResult, AnalyzeRequest } from "./index.d.mts";

export type * from "./protocol.d.mts";
export type { AdocWeaveResult, AnalyzeRequest } from "./index.d.mts";

export interface DirectAssetUrls {
  moduleUrl: string | URL;
  wasmUrl: string | URL;
}

/** 同梱WebAssemblyの位置を、この入口からの相対で求めます。 */
export declare function defaultDirectAssetUrls(baseUrl?: string | URL): {
  moduleUrl: URL;
  wasmUrl: URL;
};

export interface DirectAnalyzer {
  /**
   * 同じprocessで解析します。取消しには対応しません。WASMがtrapした場合、
   * 同じanalyzerを使い続けた結果は保証しません。新しいanalyzerを作ってください。
   */
  analyze(request: AnalyzeRequest): AdocWeaveResult;
}

/** WebAssemblyを初期化し、同じprocessで実行するanalyzerを返します。 */
export declare function createDirectAnalyzer(assets?: DirectAssetUrls): Promise<DirectAnalyzer>;

/** 既定の位置で初期化したanalyzerを使い回して解析します。 */
export declare function analyze(request: AnalyzeRequest): Promise<AdocWeaveResult>;
