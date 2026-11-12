pub(in crate::desktop) struct Template {
    pub id: &'static str,
    pub title: (&'static str, &'static str),
    pub body: (&'static str, &'static str),
}

pub(in crate::desktop) fn templates() -> &'static [Template] {
    &[
        Template {
            id: "fix-bug",
            title: ("Fix a bug", "修正錯誤"),
            body: (
                "Goal: Describe the problem and expected behavior.\nScope: Name where it happens and how to reproduce it.\nLimits: State what must remain unchanged.\nAcceptance: Describe how to verify the fix and prevent a recurrence.",
                "目標：描述問題和預期行為。\n範圍：指出發生位置及重現步驟。\n限制：說明哪些行為不能改變。\n驗收：描述如何確認修正並防止再發。",
            ),
        },
        Template {
            id: "add-feature",
            title: ("Add a feature", "新增功能"),
            body: (
                "Goal: Describe the feature and who needs it.\nScope: List the interactions and areas to change.\nLimits: Name constraints and what is out of scope.\nAcceptance: Describe observable outcomes and checks.",
                "目標：描述功能及其使用者。\n範圍：列出要修改的操作和區域。\n限制：說明限制和不包含的項目。\n驗收：描述可觀察的成果及檢查方式。",
            ),
        },
        Template {
            id: "research-question",
            title: ("Research a question", "研究問題"),
            body: (
                "Goal: Write the question and why it matters.\nScope: Specify the evidence and alternatives to examine.\nLimits: Set boundaries on sources, time, or cost.\nAcceptance: Describe the evidence and uncertainty needed for a useful answer.",
                "目標：寫出問題及其重要性。\n範圍：指定要研究的證據和替代方案。\n限制：設定來源、時間或成本的界線。\n驗收：說明有用的答案需要哪些證據和不確定性。",
            ),
        },
        Template {
            id: "refactor-safely",
            title: ("Refactor safely", "安全重構"),
            body: (
                "Goal: Describe the improvement to the code.\nScope: Identify the code to change.\nLimits: List behavior and interfaces that must stay the same.\nAcceptance: Specify checks that demonstrate the behavior is preserved.",
                "目標：描述程式碼要改善的地方。\n範圍：指出要修改的程式碼。\n限制：列出必須維持不變的行為和介面。\n驗收：指定能證明行為未變的檢查。",
            ),
        },
    ]
}
