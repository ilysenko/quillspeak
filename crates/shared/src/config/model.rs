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
        "tiny.en",
        "Tiny English",
        "ggml-tiny.en.bin",
        "75 MiB",
        78_643_200,
        "c78c86eb1a8faa21b369bcd33207cc90d64ae9df"
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
        "base.en",
        "Base English",
        "ggml-base.en.bin",
        "142 MiB",
        148_897_792,
        "137c40403d78fd54d454da0f9bd998f78703390c"
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
        "small.en",
        "Small English",
        "ggml-small.en.bin",
        "466 MiB",
        488_636_416,
        "db8a495a91d927739e50b3fc1cc4c6b8f6c2d022"
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
        "medium.en",
        "Medium English",
        "ggml-medium.en.bin",
        "1.5 GiB",
        1_610_612_736,
        "8c30f0e44ce9560643ebd10bbe50cd20eafd3723"
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
];

pub fn model_catalog_entry(model_id: &str) -> Option<ModelCatalogEntry> {
    MODEL_CATALOG
        .iter()
        .copied()
        .find(|entry| entry.id == model_id)
}
