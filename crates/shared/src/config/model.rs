#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelCatalogEntry {
    pub id: &'static str,
    pub label: &'static str,
    pub filename: &'static str,
    pub size_label: &'static str,
    pub size_bytes: u64,
    pub sha1: &'static str,
    pub url: &'static str,
}

pub const DEFAULT_MODEL_ID: &str = "large-v3-turbo-q5_0";

macro_rules! model {
    ($id:literal, $label:literal, $filename:literal, $size_label:literal, $size_bytes:literal, $sha1:literal) => {
        ModelCatalogEntry {
            id: $id,
            label: $label,
            filename: $filename,
            size_label: $size_label,
            size_bytes: $size_bytes,
            sha1: $sha1,
            url: concat!(
                "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/",
                $filename
            ),
        }
    };
}

pub const MODEL_CATALOG: &[ModelCatalogEntry] = &[
    model!(
        "tiny",
        "Tiny",
        "ggml-tiny.bin",
        "75 MiB",
        78_643_200,
        "bd577a113a864445d4c299885e0cb97d4ba92b5f"
    ),
    model!(
        "tiny-q5_1",
        "Tiny Q5",
        "ggml-tiny-q5_1.bin",
        "31 MiB",
        32_152_673,
        "2827a03e495b1ed3048ef28a6a4620537db4ee51"
    ),
    model!(
        "tiny-q8_0",
        "Tiny Q8",
        "ggml-tiny-q8_0.bin",
        "42 MiB",
        43_537_433,
        "19e8118f6652a650569f5a949d962154e01571d9"
    ),
    model!(
        "tiny.en",
        "Tiny English",
        "ggml-tiny.en.bin",
        "75 MiB",
        78_643_200,
        "c78c86eb1a8faa21b369bcd33207cc90d64ae9df"
    ),
    model!(
        "tiny.en-q5_1",
        "Tiny English Q5",
        "ggml-tiny.en-q5_1.bin",
        "31 MiB",
        32_166_155,
        "3fb92ec865cbbc769f08137f22470d6b66e071b6"
    ),
    model!(
        "tiny.en-q8_0",
        "Tiny English Q8",
        "ggml-tiny.en-q8_0.bin",
        "42 MiB",
        43_550_795,
        "802d6668e7d411123e672abe4cb6c18f12306abb"
    ),
    model!(
        "base",
        "Base",
        "ggml-base.bin",
        "142 MiB",
        148_897_792,
        "465707469ff3a37a2b9b8d8f89f2f99de7299dac"
    ),
    model!(
        "base-q5_1",
        "Base Q5",
        "ggml-base-q5_1.bin",
        "57 MiB",
        59_707_625,
        "a3733eda680ef76256db5fc5dd9de8629e62c5e7"
    ),
    model!(
        "base-q8_0",
        "Base Q8",
        "ggml-base-q8_0.bin",
        "78 MiB",
        81_768_585,
        "7bb89bb49ed6955013b166f1b6a6c04584a20fbe"
    ),
    model!(
        "base.en",
        "Base English",
        "ggml-base.en.bin",
        "142 MiB",
        148_897_792,
        "137c40403d78fd54d454da0f9bd998f78703390c"
    ),
    model!(
        "base.en-q5_1",
        "Base English Q5",
        "ggml-base.en-q5_1.bin",
        "57 MiB",
        59_721_011,
        "d26d7ce5a1b6e57bea5d0431b9c20ae49423c94a"
    ),
    model!(
        "base.en-q8_0",
        "Base English Q8",
        "ggml-base.en-q8_0.bin",
        "78 MiB",
        81_781_811,
        "bb1574182e9b924452bf0cd1510ac034d323e948"
    ),
    model!(
        "small",
        "Small",
        "ggml-small.bin",
        "466 MiB",
        488_636_416,
        "55356645c2b361a969dfd0ef2c5a50d530afd8d5"
    ),
    model!(
        "small-q5_1",
        "Small Q5",
        "ggml-small-q5_1.bin",
        "181 MiB",
        190_085_487,
        "6fe57ddcfdd1c6b07cdcc73aaf620810ce5fc771"
    ),
    model!(
        "small-q8_0",
        "Small Q8",
        "ggml-small-q8_0.bin",
        "252 MiB",
        264_464_607,
        "bcad8a2083f4e53d648d586b7dbc0cd673d8afad"
    ),
    model!(
        "small.en",
        "Small English",
        "ggml-small.en.bin",
        "466 MiB",
        488_636_416,
        "db8a495a91d927739e50b3fc1cc4c6b8f6c2d022"
    ),
    model!(
        "small.en-q5_1",
        "Small English Q5",
        "ggml-small.en-q5_1.bin",
        "181 MiB",
        190_098_681,
        "20f54878d608f94e4a8ee3ae56016571d47cba34"
    ),
    model!(
        "small.en-q8_0",
        "Small English Q8",
        "ggml-small.en-q8_0.bin",
        "252 MiB",
        264_477_561,
        "9d75ff4ccfa0a8217870d7405cf8cef0a5579852"
    ),
    model!(
        "medium",
        "Medium",
        "ggml-medium.bin",
        "1.5 GiB",
        1_610_612_736,
        "fd9727b6e1217c2f614f9b698455c4ffd82463b4"
    ),
    model!(
        "medium-q5_0",
        "Medium Q5",
        "ggml-medium-q5_0.bin",
        "514 MiB",
        539_212_467,
        "7718d4c1ec62ca96998f058114db98236937490e"
    ),
    model!(
        "medium-q8_0",
        "Medium Q8",
        "ggml-medium-q8_0.bin",
        "785 MiB",
        823_369_779,
        "e66645948aff4bebbec71b3485c576f3d63af5d6"
    ),
    model!(
        "medium.en",
        "Medium English",
        "ggml-medium.en.bin",
        "1.5 GiB",
        1_610_612_736,
        "8c30f0e44ce9560643ebd10bbe50cd20eafd3723"
    ),
    model!(
        "medium.en-q5_0",
        "Medium English Q5",
        "ggml-medium.en-q5_0.bin",
        "514 MiB",
        539_225_533,
        "bb3b5281bddd61605d6fc76bc5b92d8f20284c3b"
    ),
    model!(
        "medium.en-q8_0",
        "Medium English Q8",
        "ggml-medium.en-q8_0.bin",
        "785 MiB",
        823_382_461,
        "b1cf48c12c807e14881f634fb7b6c6ca867f6b38"
    ),
    model!(
        "large-v1",
        "Large v1",
        "ggml-large-v1.bin",
        "2.9 GiB",
        3_094_623_691,
        "b1caaf735c4cc1429223d5a74f0f4d0b9b59a299"
    ),
    model!(
        "large-v2",
        "Large v2",
        "ggml-large-v2.bin",
        "2.9 GiB",
        3_094_623_691,
        "0f4c8e34f21cf1a914c59d8b3ce882345ad349d6"
    ),
    model!(
        "large-v2-q5_0",
        "Large v2 Q5",
        "ggml-large-v2-q5_0.bin",
        "1.1 GiB",
        1_080_732_091,
        "00e39f2196344e901b3a2bd5814807a769bd1630"
    ),
    model!(
        "large-v2-q8_0",
        "Large v2 Q8",
        "ggml-large-v2-q8_0.bin",
        "1.5 GiB",
        1_656_129_691,
        "da97d6ca8f8ffbeeb5fd147f79010eeea194ba38"
    ),
    model!(
        "large-v3",
        "Large v3",
        "ggml-large-v3.bin",
        "2.9 GiB",
        3_113_877_504,
        "ad82bf6a9043ceed055076d0fd39f5f186ff8062"
    ),
    model!(
        "large-v3-q5_0",
        "Large v3 Q5",
        "ggml-large-v3-q5_0.bin",
        "1.1 GiB",
        1_181_116_006,
        "e6e2ed78495d403bef4b7cff42ef4aaadcfea8de"
    ),
    model!(
        "large-v3-turbo",
        "Large v3 Turbo",
        "ggml-large-v3-turbo.bin",
        "1.5 GiB",
        1_610_612_736,
        "4af2b29d7ec73d781377bfd1758ca957a807e941"
    ),
    model!(
        "large-v3-turbo-q5_0",
        "Large v3 Turbo Q5",
        "ggml-large-v3-turbo-q5_0.bin",
        "547 MiB",
        573_571_072,
        "e050f7970618a659205450ad97eb95a18d69c9ee"
    ),
    model!(
        "large-v3-turbo-q8_0",
        "Large v3 Turbo Q8",
        "ggml-large-v3-turbo-q8_0.bin",
        "834 MiB",
        874_188_075,
        "01bf15bedffe9f39d65c1b6ff9b687ea91f59e0e"
    ),
];

pub fn model_catalog_entry(model_id: &str) -> Option<ModelCatalogEntry> {
    MODEL_CATALOG
        .iter()
        .copied()
        .find(|entry| entry.id == model_id)
}
