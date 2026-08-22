import { createPositionMapper } from "./position.mjs";

export function materializeTxtAST(source, plan) {
  if (typeof source !== "string") {
    throw new TypeError("AsciiDocの入力は文字列で指定してください。");
  }
  const positions = createPositionMapper(source);

  function materialize(node) {
    const {
      type,
      range: plannedRange,
      valueRange,
      children,
      raw: _raw,
      value: _value,
      loc: _loc,
      ...properties
    } = node;
    const range = positions.assertRange(plannedRange, `${type}のrange`);
    const result = { type, ...positions.base(range), ...properties };

    if (valueRange !== undefined) {
      const value = positions.assertRange(valueRange, `${type}のvalueRange`);
      result.value = source.slice(value[0], value[1]);
    }
    if (children !== undefined) {
      result.children = children.map(materialize);
    }
    return result;
  }

  return materialize(plan);
}
