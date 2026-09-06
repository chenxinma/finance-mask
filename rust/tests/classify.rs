//! classify 集成测试 —— 对应 task-2-brief Step 2.1。
//!
//! 夹具来自 layout-view/files（data1-6, form1-3, test_data），
//! 断言与 layout-view 现行源码（被吸收的 lib.rs）一致。
//!
//! 已知分歧：form3.xlsx 在 Python oracle 所用的 lib/layout_view.dll（未提交变体）
//! 下分类为 Data，本实现为 Form —— 两条路径产出相同位点（见 classify.rs 头注释）。

use finance_mask_core::classify::{classify_excel_sheets, SheetType};

/// brief Step 2.1：data 系夹具含 Data 表，form 系夹具不含 Data 表。
#[test]
fn layout_view_fixtures_classify_as_expected() {
    for f in ["data1", "data3", "data6"] {
        let r = classify_excel_sheets(&format!("tests/fixtures/{f}.xlsx")).unwrap();
        assert!(
            r.iter().any(|s| s.sheet_type == SheetType::Data),
            "{f} 应含 Data 工作表"
        );
    }
    for f in ["form1", "form2", "form3"] {
        let r = classify_excel_sheets(&format!("tests/fixtures/{f}.xlsx")).unwrap();
        assert!(
            r.iter().all(|s| s.sheet_type != SheetType::Data),
            "{f} 不应含 Data 工作表"
        );
    }
}

/// 分类顺序 = 工作簿 sheet 顺序；隐藏表（data1/转移、form1/DropDown 等）被跳过，
/// density=0 的空表（form2/Sheet2）也被过滤（layout-view classify_excel_sheets 语义）。
#[test]
fn classification_skips_hidden_and_empty_sheets_in_workbook_order() {
    let r = classify_excel_sheets("tests/fixtures/data1.xlsx").unwrap();
    assert_eq!(
        r.iter().map(|s| s.original.sheet_name.as_str()).collect::<Vec<_>>(),
        vec!["增员", "减员"],
        "隐藏表「转移」应被跳过"
    );

    let r = classify_excel_sheets("tests/fixtures/form2.xlsx").unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].original.sheet_name, "Sheet1", "空表 Sheet2 应被过滤");

    let r = classify_excel_sheets("tests/fixtures/form1.xlsx").unwrap();
    assert_eq!(r.len(), 1, "form1 仅 1 张可见非空表");
    assert_eq!(r[0].original.sheet_name, "New Employee Data Collection Fo");
}

/// 差分夹具全集的稳定分类（data 系为 Data，form 系为 Form）——
/// 一旦 calamine/算法行为漂移，此测试先于差分测试报警。
#[test]
fn differential_fixture_classifications_are_stable() {
    let expect: &[(&str, &[(&str, SheetType)])] = &[
        ("data2.xlsx", &[("入职", SheetType::Data), ("离职", SheetType::Data)]),
        ("data4.xlsx", &[
            ("ArgoDB权限统计", SheetType::Data),
            ("ArgoDB PT 权限", SheetType::Data),
            ("Trino 权限统计", SheetType::Data),
        ]),
        ("data5.xlsx", &[("AI财有道", SheetType::Data), ("AI底座", SheetType::Data)]),
        ("data6.xlsx", &[("合同报价", SheetType::Data), ("采购计划", SheetType::Data)]),
        ("test_data.xlsx", &[
            ("Data Sheet", SheetType::Data),
            ("Form Sheet", SheetType::Data),
            ("Sparse Sheet", SheetType::Data),
        ]),
        ("multi_header.xlsx", &[("财务报表", SheetType::Data)]),
        ("single_header.xlsx", &[("利润表", SheetType::Data)]),
    ];
    for (file, want) in expect {
        let r = classify_excel_sheets(&format!("tests/fixtures/{file}")).unwrap();
        let got: Vec<(&str, &SheetType)> = r
            .iter()
            .map(|s| (s.original.sheet_name.as_str(), &s.sheet_type))
            .collect();
        let want: Vec<(&str, &SheetType)> = want.iter().map(|(n, t)| (*n, t)).collect();
        assert_eq!(got, want, "{file}");
    }
}
