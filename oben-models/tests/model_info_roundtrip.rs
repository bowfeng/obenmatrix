use oben_models::{ModelInfo, ModelListResponse};

#[test]
fn model_info_roundtrip_json() {
    let model = ModelInfo {
        id: "qwen35-local".to_string(),
        object: "model".to_string(),
        created: 1779066270,
        owned_by: "vllm".to_string(),
        max_model_len: Some(262144),
        root: Some("/models".to_string()),
        parent: None,
    };

    let json = serde_json::to_string(&model).unwrap();
    let decoded: ModelInfo = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.id, "qwen35-local");
    assert_eq!(decoded.max_model_len, Some(262144));
    assert_eq!(decoded.owned_by, "vllm");
    assert_eq!(decoded.root, Some("/models".to_string()));
}

#[test]
fn model_info_missing_optional_fields_json() {
    let json = r#"{
        "id": "gpt-4",
        "object": "model",
        "created": 1234567890,
        "owned_by": "openai"
    }"#;
    let model: ModelInfo = serde_json::from_str(json).unwrap();
    assert_eq!(model.id, "gpt-4");
    assert_eq!(model.max_model_len, None);
    assert_eq!(model.root, None);
    assert_eq!(model.parent, None);
}

#[test]
fn model_list_response_roundtrip_json() {
    let response = ModelListResponse {
        object: "list".to_string(),
        data: vec![
            ModelInfo {
                id: "qwen35-local".to_string(),
                object: "model".to_string(),
                created: 1779066270,
                owned_by: "vllm".to_string(),
                max_model_len: Some(262144),
                root: Some("/models".to_string()),
                parent: None,
            },
            ModelInfo {
                id: "gpt-4".to_string(),
                object: "model".to_string(),
                created: 1234567890,
                owned_by: "openai".to_string(),
                max_model_len: Some(128000),
                root: None,
                parent: None,
            },
        ],
    };

    let json = serde_json::to_string(&response).unwrap();
    let decoded: ModelListResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.data.len(), 2);
    assert_eq!(decoded.data[0].max_model_len, Some(262144));
    assert_eq!(decoded.data[1].max_model_len, Some(128000));
}

#[test]
fn model_info_roundtrip_yaml() {
    let model = ModelInfo {
        id: "qwen35-local".to_string(),
        object: "model".to_string(),
        created: 1779066270,
        owned_by: "vllm".to_string(),
        max_model_len: Some(262144),
        root: Some("/models".to_string()),
        parent: None,
    };

    let yaml = serde_yaml::to_string(&model).unwrap();
    let decoded: ModelInfo = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(decoded.id, "qwen35-local");
    assert_eq!(decoded.max_model_len, Some(262144));
}
