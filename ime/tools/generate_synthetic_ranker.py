"""生成仅用于 ModelHost 契约测试的微型 ONNX 候选排序模型。"""

from pathlib import Path
import sys

import onnx
from onnx import TensorProto, helper


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: generate_synthetic_ranker.py <output.onnx>", file=sys.stderr)
        return 2

    batch = "batch"
    sequence_length = 64
    inputs = [
        helper.make_tensor_value_info("input_ids", TensorProto.INT64, [batch, sequence_length]),
        helper.make_tensor_value_info("attention_mask", TensorProto.INT64, [batch, sequence_length]),
        helper.make_tensor_value_info("token_type_ids", TensorProto.INT64, [batch, sequence_length]),
    ]
    output = helper.make_tensor_value_info("logits", TensorProto.FLOAT, [batch, 1])
    axes = helper.make_tensor("axes", TensorProto.INT64, [1], [1])
    type_weight = helper.make_tensor("type_weight", TensorProto.FLOAT, [1], [0.001])
    nodes = [
        helper.make_node("Cast", ["input_ids"], ["input_ids_f"], to=TensorProto.FLOAT),
        helper.make_node("Cast", ["attention_mask"], ["attention_mask_f"], to=TensorProto.FLOAT),
        helper.make_node("Cast", ["token_type_ids"], ["token_type_ids_f"], to=TensorProto.FLOAT),
        helper.make_node("Mul", ["input_ids_f", "attention_mask_f"], ["masked_ids"]),
        helper.make_node("Mul", ["token_type_ids_f", "type_weight"], ["weighted_types"]),
        helper.make_node("Add", ["masked_ids", "weighted_types"], ["features"]),
        helper.make_node("ReduceSum", ["features", "axes"], ["logits"], keepdims=1),
    ]
    graph = helper.make_graph(nodes, "owo.synthetic.rank.v1", inputs, [output], [axes, type_weight])
    model = helper.make_model(
        graph,
        producer_name="OwO.InputMethod",
        producer_version="1",
        opset_imports=[helper.make_opsetid("", 17)],
        ir_version=10,
    )
    onnx.checker.check_model(model, full_check=True)
    output_path = Path(sys.argv[1])
    output_path.parent.mkdir(parents=True, exist_ok=True)
    onnx.save_model(model, output_path)
    print(f"wrote {output_path} ({output_path.stat().st_size} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
