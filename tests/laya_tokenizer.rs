use axiom::laya::tokenizer::Tokenizer;
use serde_json::{json, Value};
use std::path::Path;
const WORD: &str = include_str!("fixtures/laya_tokenizer/wordpiece.json");
const BPE: &str = include_str!("fixtures/laya_tokenizer/bpe.json");
fn value(s: &str) -> Value {
    serde_json::from_str(s.trim_start_matches('\u{feff}')).unwrap()
}
fn tokenizer(v: &Value) -> Tokenizer {
    Tokenizer::from_str(&v.to_string()).unwrap()
}
#[test]
fn wordpiece_profile() {
    let t =
        Tokenizer::from_file(Path::new("tests/fixtures/laya_tokenizer/wordpiece.json")).unwrap();
    assert_eq!(
        t.encode("Hello, WORLD! playing").unwrap(),
        vec![5, 8, 6, 7, 9, 10]
    );
    assert_eq!(t.encode("Café \u{0000} X").unwrap(), vec![11, 13]);
    assert_eq!(
        t.encode("中").unwrap_err().kind,
        "laya_tokenizer_unsupported"
    );
    let mut chinese = value(WORD);
    chinese["normalizer"]["strip_accents"] = json!(false);
    assert_eq!(tokenizer(&chinese).encode("中").unwrap(), vec![12]);
    assert_eq!(t.encode("he\u{200b}llo ca\u{05b0}fe").unwrap(), vec![5, 11]);
    assert_eq!(t.encode("mystery abcdefghijklm").unwrap(), vec![1, 1]);
    assert_eq!(t.token_to_id("##ing"), Some(10));
    assert_eq!(t.token_to_id("missing"), None);
    let s = t.special();
    assert_eq!((s.cls, s.sep, s.pad, s.mask, s.unk), (2, 3, 0, 4, 1));
    assert_ne!(t.encode("Hello!").unwrap(), vec![5, 8]);
    assert_eq!(t.encode("Hello!").unwrap(), t.encode("Hello!").unwrap());
    fn send_sync<T: Send + Sync>() {}
    send_sync::<Tokenizer>();
}
#[test]
fn bpe_and_bytes() {
    let mut v = value(BPE);
    let t = tokenizer(&v);
    assert_eq!(t.encode("abc é").unwrap(), vec![9, 10, 11]);
    assert_eq!(t.encode("abcabc").unwrap(), vec![9, 9]);
    v["model"]["ignore_merges"] = json!(true);
    assert_eq!(tokenizer(&v).encode("abcabc").unwrap(), vec![16]);
    v["model"]["ignore_merges"] = json!(false);
    v["pre_tokenizer"] = json!({"type":"ByteLevel","add_prefix_space":true,"use_regex":true});
    assert_eq!(tokenizer(&v).encode("a a").unwrap(), vec![13, 13]);
    v["pre_tokenizer"] =
        json!({"type":"Metaspace","replacement":"▁","prepend_scheme":"always","split":true});
    assert_eq!(tokenizer(&v).encode("a a").unwrap(), vec![15, 15]);
}
#[test]
fn added_tokens() {
    let t = tokenizer(&value(WORD));
    assert_eq!(
        t.encode("HELLO[MASK]<TAG>world").unwrap(),
        vec![5, 4, 14, 6]
    );
    let mut v = value(WORD);
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":15,"content":"NEW","special":false,"single_word":true}));
    let t = tokenizer(&v);
    assert_eq!(t.encode("New newer").unwrap(), vec![15, 1]);
}
#[test]
fn rejects_unsupported_and_missing() {
    let mut v = value(WORD);
    v["normalizer"] = json!({"type":"Strip"});
    let e = Tokenizer::from_str(&v.to_string()).err().unwrap();
    assert_eq!(e.kind, "laya_tokenizer_unsupported");
    assert!(e.message.contains("Strip"));
    v = value(WORD);
    v["pre_tokenizer"] = json!({"type":"Digits"});
    let e = Tokenizer::from_str(&v.to_string()).err().unwrap();
    assert_eq!(e.kind, "laya_tokenizer_unsupported");
    assert!(e.message.contains("Digits"));
    v = value(WORD);
    v["model"] = json!({"type":"Unigram","vocab":[]});
    let e = Tokenizer::from_str(&v.to_string()).err().unwrap();
    assert_eq!(e.kind, "laya_tokenizer_unsupported");
    assert!(e.message.contains("Unigram"));
    v = value(WORD);
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["content"] != "[MASK]");
    assert_eq!(
        Tokenizer::from_str(&v.to_string()).err().unwrap().kind,
        "laya_tokenizer_invalid"
    );
    v = value(WORD);
    v["normalizer"] = json!({"type":"NFC"});
    let t = tokenizer(&v);
    assert_eq!(
        t.encode("e\u{301}").unwrap_err().kind,
        "laya_tokenizer_invalid"
    );
    assert_eq!(
        t.encode("\u{1100}").unwrap_err().kind,
        "laya_tokenizer_invalid"
    );
}
#[test]
fn sequences_replace_split_and_prepend() {
    let mut v = value(WORD);
    v["normalizer"] = json!({"type":"Sequence","normalizers":[{"type":"Replace","pattern":{"String":"X"},"content":"hello"},{"type":"Prepend","prepend":"x "},{"type":"Lowercase"}]});
    v["pre_tokenizer"] = json!({"type":"Sequence","pretokenizers":[{"type":"Split","pattern":{"String":"/"},"behavior":"Removed","invert":false},{"type":"Whitespace"}]});
    assert_eq!(tokenizer(&v).encode("X/WORLD!").unwrap(), vec![13, 5, 6, 7]);
}

#[test]
fn template_ids_and_added_special_resolution() {
    let mut v = value(BPE);
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .retain(|a| a["content"] != "[CLS]" && a["content"] != "[SEP]");
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|a| a["content"] == "[PAD]")
        .unwrap()["special"] = json!(false);
    let t = tokenizer(&v);
    assert_eq!(
        (t.special().cls, t.special().sep, t.special().pad),
        (2, 3, 0)
    );
}

#[test]
fn added_tokens_do_not_extend_model_vocabulary() {
    let mut v = value(BPE);
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":17,"content":"z","single_word":true,"special":false}));
    let t = tokenizer(&v);
    assert_eq!(t.token_to_id("z"), Some(17));
    assert_eq!(t.encode("z az").unwrap(), vec![17, 5, 1]);
}

#[test]
fn template_uses_named_special_token_ids() {
    let mut v = value(BPE);
    v["post_processor"] = json!({"type":"TemplateProcessing", "single":"<bos> $A <eos>",
        "special_tokens":{"<bos>":{"ids":[42]}, "<eos>":{"ids":[43]}}});
    let t = tokenizer(&v);
    assert_eq!((t.special().cls, t.special().sep), (42, 43));
    assert_eq!(t.encode("abc").unwrap(), vec![9]);
}

#[test]
fn bpe_unknown_and_fuse_unknown() {
    let mut v = value(BPE);
    v["model"]["byte_fallback"] = json!(false);
    assert_eq!(tokenizer(&v).encode("aé☃b").unwrap(), vec![5, 1, 1, 6]);
    v["model"]["fuse_unk"] = json!(true);
    assert_eq!(tokenizer(&v).encode("aé☃b☃").unwrap(), vec![5, 1, 6, 1]);
}

#[test]
fn split_behaviors_and_invert() {
    let mut v = value(BPE);
    v["model"]["ignore_merges"] = json!(true);
    for (token, id) in [("/", 20), ("//", 21), ("a//", 22), ("//b", 23), ("/b", 24)] {
        v["model"]["vocab"][token] = json!(id);
    }
    for (behavior, invert, expected) in [
        ("Isolated", false, vec![5, 20, 20, 6]),
        ("MergedWithPrevious", false, vec![22, 6]),
        ("MergedWithNext", false, vec![5, 23]),
        ("Contiguous", false, vec![5, 21, 6]),
        ("MergedWithPrevious", true, vec![5, 20, 24]),
    ] {
        v["pre_tokenizer"] =
            json!({"type":"Split", "pattern":{"String":"/"}, "behavior":behavior, "invert":invert});
        assert_eq!(
            tokenizer(&v).encode("a//b").unwrap(),
            expected,
            "{behavior}, invert={invert}"
        );
    }
}

#[test]
fn metaspace_schemes_and_no_split() {
    let mut v = value(BPE);
    v["model"]["ignore_merges"] = json!(true);
    v["model"]["vocab"]["a▁a"] = json!(20);
    v["pre_tokenizer"] =
        json!({"type":"Metaspace", "replacement":"▁", "prepend_scheme":"never", "split":true});
    assert_eq!(tokenizer(&v).encode("a a").unwrap(), vec![5, 15]);
    v["pre_tokenizer"]["split"] = json!(false);
    assert_eq!(tokenizer(&v).encode("a a").unwrap(), vec![20]);
    v["pre_tokenizer"] = json!({"type":"Sequence", "pretokenizers":[
        {"type":"Split", "pattern":{"String":"/"}, "behavior":"Removed", "invert":false},
        {"type":"Metaspace", "replacement":"▁", "prepend_scheme":"first", "split":true}
    ]});
    assert_eq!(tokenizer(&v).encode("a/a").unwrap(), vec![15, 5]);
}

#[test]
fn bytelevel_without_regex_keeps_one_chunk() {
    let mut v = value(BPE);
    v["model"]["ignore_merges"] = json!(true);
    v["model"]["vocab"]["aĠa"] = json!(20);
    v["pre_tokenizer"] = json!({"type":"ByteLevel", "add_prefix_space":false, "use_regex":false});
    assert_eq!(tokenizer(&v).encode("a a").unwrap(), vec![20]);
    v["pre_tokenizer"]["use_regex"] = json!(true);
    assert_eq!(tokenizer(&v).encode("a a").unwrap(), vec![5, 13]);
}

#[test]
fn added_tokens_strip_surrounding_whitespace() {
    let mut v = value(BPE);
    v["pre_tokenizer"] = serde_json::Value::Null;
    v["model"]["byte_fallback"] = json!(false);
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":20, "content":"<X>", "lstrip":true, "rstrip":true}));
    assert_eq!(tokenizer(&v).encode("a  <X> \tb").unwrap(), vec![5, 20, 6]);
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["lstrip"] = json!(false);
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["rstrip"] = json!(false);
    assert_eq!(
        tokenizer(&v).encode("a  <X> \tb").unwrap(),
        vec![5, 1, 1, 20, 1, 1, 6]
    );
}

#[test]
fn rejects_bpe_dropout() {
    let mut v = value(BPE);
    v["model"]["dropout"] = json!(0.1);
    assert_eq!(
        Tokenizer::from_str(&v.to_string()).err().unwrap().kind,
        "laya_tokenizer_unsupported"
    );
}

#[test]
fn bert_accent_table_and_unsupported_letters() {
    let mut v = value(WORD);
    v["model"]["vocab"]["scz"] = json!(20);
    assert_eq!(tokenizer(&v).encode("ŚČŽ").unwrap(), vec![20]);
    let t = tokenizer(&v);
    for letter in ["ạ", "й", "ά", "한", "中"] {
        assert_eq!(
            t.encode(letter).unwrap_err().kind,
            "laya_tokenizer_unsupported",
            "{letter}"
        );
    }
}

#[test]
fn nfc_rejects_non_nfc_singletons() {
    let mut v = value(WORD);
    v["normalizer"] = json!({"type":"NFC"});
    let t = tokenizer(&v);
    for c in ['\u{2126}', '\u{212a}', '\u{212b}'] {
        assert_eq!(
            t.encode(&c.to_string()).unwrap_err().kind,
            "laya_tokenizer_invalid",
            "{c}"
        );
    }
}

#[test]
fn normalized_added_token_errors_at_load() {
    let mut v = value(WORD);
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .push(json!({"id":20, "content":"ạ", "normalized":true}));
    assert_eq!(
        Tokenizer::from_str(&v.to_string()).err().unwrap().kind,
        "laya_tokenizer_unsupported"
    );
    v["added_tokens"]
        .as_array_mut()
        .unwrap()
        .last_mut()
        .unwrap()["normalized"] = json!(false);
    assert_eq!(tokenizer(&v).encode("ạ").unwrap(), vec![20]);
}

#[test]
fn accepts_leading_utf8_bom() {
    let t = Tokenizer::from_str(&format!("\u{feff}{WORD}")).unwrap();
    assert_eq!(t.encode("Hello").unwrap(), vec![5]);
}
