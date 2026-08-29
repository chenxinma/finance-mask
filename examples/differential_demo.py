"""差分脱敏模式演示"""
from decimal import Decimal
from finance_mask.engine.amount import AmountRedactor


def demo_differential_shift():
    """演示差分偏移模式"""
    print("=" * 60)
    print("差分偏移模式演示 (differential_shift)")
    print("=" * 60)
    print()
    print("原理：所有金额加上相同的固定偏移量，保持差值不变")
    print()
    
    # 示例数据：资产负债表
    values = [
        "10000000",      # 资产
        "6000000",       # 负债
        "4000000",       # 所有者权益
    ]
    labels = ["资产", "负债", "所有者权益"]
    
    print("原始数据：")
    for label, value in zip(labels, values):
        print(f"  {label}: {float(value):,.2f} 元")
    
    print()
    print(f"资产 - 负债 = 所有者权益: {float(values[0]):,.2f} - {float(values[1]):,.2f} = {float(values[2]):,.2f}")
    print()
    
    # 生成差分序列
    results, shift = AmountRedactor.generate_differential_sequence(
        values,
        shift_range=(500000, 1500000),  # 偏移50万-150万
        seed=42,
    )
    
    print(f"生成的偏移量: {shift:,.2f} 元")
    print()
    print("脱敏后数据：")
    for label, original, result in zip(labels, values, results):
        print(f"  {label}: {result} 元")
    
    # 验证差值保持不变
    original_diff1 = float(values[0]) - float(values[1])
    result_diff1 = float(results[0].replace(",", "")) - float(results[1].replace(",", ""))
    
    print()
    print("验证差值保持不变：")
    print(f"  原始差值 (资产-负债): {original_diff1:,.2f}")
    print(f"  脱敏后差值: {result_diff1:,.2f}")
    print(f"  差值差异: {abs(original_diff1 - result_diff1):.2f} (应为0)")


def demo_proportional_scale():
    """演示比例缩放模式"""
    print()
    print("=" * 60)
    print("比例缩放模式演示 (proportional_scale)")
    print("=" * 60)
    print()
    print("原理：所有金额乘以相同的比例因子，保持比例关系不变")
    print()
    
    # 示例数据：收入和成本
    values = [
        "10000000",      # 营业收入
        "7000000",       # 营业成本
        "3000000",       # 毛利润
    ]
    labels = ["营业收入", "营业成本", "毛利润"]
    
    print("原始数据：")
    for label, value in zip(labels, values):
        print(f"  {label}: {float(value):,.2f} 元")
    
    # 计算原始毛利率
    gross_margin = float(values[2]) / float(values[0])
    print()
    print(f"原始毛利率: {gross_margin:.2%}")
    print()
    
    # 生成比例序列
    results, scale = AmountRedactor.generate_proportional_differential_sequence(
        values,
        percentage=15.0,  # ±15% 范围
        seed=42,
    )
    
    print(f"生成的缩放因子: {scale:.4f}")
    print()
    print("脱敏后数据：")
    for label, original, result in zip(labels, values, results):
        print(f"  {label}: {result} 元")
    
    # 验证比例保持不变
    result_margin = float(results[2].replace(",", "")) / float(results[0].replace(",", ""))
    print()
    print("验证比例关系保持不变：")
    print(f"  原始毛利率: {gross_margin:.2%}")
    print(f"  脱敏后毛利率: {result_margin:.2%}")
    print(f"  比例差异: {abs(gross_margin - result_margin):.6f} (应接近0)")


def demo_comparison():
    """对比三种扰动模式"""
    print()
    print("=" * 60)
    print("三种扰动模式对比")
    print("=" * 60)
    print()
    
    values = ["1000000", "2000000", "3000000"]
    labels = ["金额A", "金额B", "金额C"]
    
    print("原始数据：")
    for label, value in zip(labels, values):
        print(f"  {label}: {float(value):,.2f} 元")
    
    print()
    print(f"原始差值: B-A = {float(values[1]) - float(values[0]):,.2f}, C-B = {float(values[2]) - float(values[1]):,.2f}")
    print(f"原始比例: B/A = {float(values[1])/float(values[0]):.2f}, C/B = {float(values[2])/float(values[1]):.2f}")
    print()
    
    # 1. 随机扰动（保持总量）
    import random
    random.seed(42)
    results_perturb = AmountRedactor.generate_perturbation_sequence(values, percentage=10.0)
    print("1. 随机扰动模式 (perturb) - 保持总量不变：")
    for label, result in zip(labels, results_perturb):
        print(f"     {label}: {result}")
    total_orig = sum(float(v) for v in values)
    total_perturb = sum(float(r.replace(",", "")) for r in results_perturb)
    print(f"     总量: {total_perturb:,.2f} (原始: {total_orig:,.2f})")
    print()
    
    # 2. 差分偏移（保持差值）
    results_shift, shift = AmountRedactor.generate_differential_sequence(values, seed=42)
    print("2. 差分偏移模式 (differential_shift) - 保持差值不变：")
    for label, result in zip(labels, results_shift):
        print(f"     {label}: {result}")
    diff1 = float(results_shift[1].replace(",", "")) - float(results_shift[0].replace(",", ""))
    diff2 = float(results_shift[2].replace(",", "")) - float(results_shift[1].replace(",", ""))
    print(f"     差值: B-A = {diff1:,.2f}, C-B = {diff2:,.2f}")
    print()
    
    # 3. 比例缩放（保持比例）
    results_scale, scale = AmountRedactor.generate_proportional_differential_sequence(values, percentage=10.0, seed=42)
    print("3. 比例缩放模式 (proportional_scale) - 保持比例不变：")
    for label, result in zip(labels, results_scale):
        print(f"     {label}: {result}")
    ratio1 = float(results_scale[1].replace(",", "")) / float(results_scale[0].replace(",", ""))
    ratio2 = float(results_scale[2].replace(",", "")) / float(results_scale[1].replace(",", ""))
    print(f"     比例: B/A = {ratio1:.2f}, C/B = {ratio2:.2f}")


if __name__ == "__main__":
    demo_differential_shift()
    demo_proportional_scale()
    demo_comparison()
