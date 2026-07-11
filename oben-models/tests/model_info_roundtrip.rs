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

#[test]
fn model_info_deserialize_qwen_format_with_meta_n_ctx() {
    // Qwen/Ollama format with meta.n_ctx instead of max_model_len
    let json = r#"{
        "name": "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M",
        "model": "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M",
        "id": "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M",
        "size": 48405005312,
        "meta": {"n_ctx": 262144}
    }"#;
    let model: ModelInfo = serde_json::from_str(json).unwrap();
    assert_eq!(model.id, "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M");
    assert_eq!(model.max_model_len, Some(262144));
    assert_eq!(model.owned_by, "unknown");
}

#[test]
fn model_list_response_deserialize_qwen_format_with_models_array() {
    // Qwen format with "models" array instead of "data"
    let json = r#"{
        "models": [{
            "name": "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M",
            "model": "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M",
            "id": "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M",
            "size": 48405005312,
            "meta": {"n_ctx": 262144}
        }]
    }"#;
    let response: ModelListResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.object, "list");
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.data[0].id, "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M");
    assert_eq!(response.data[0].max_model_len, Some(262144));
}

#[test]
fn model_list_response_deserialize_openai_format_with_data_array() {
    // OpenAI format with "data" array
    let json = r#"{
        "object": "list",
        "data": [{
            "id": "gpt-4",
            "object": "model",
            "created": 1234567890,
            "owned_by": "openai",
            "max_model_len": 128000
        }]
    }"#;
    let response: ModelListResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.object, "list");
    assert_eq!(response.data.len(), 1);
    assert_eq!(response.data[0].id, "gpt-4");
    assert_eq!(response.data[0].max_model_len, Some(128000));
}

#[test]
fn model_list_response_deserialize_both_arrays_merged() {
    let json = r#"{
        "object": "list",
        "data": [{
            "id": "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M",
            "object": "model",
            "created": 1783771267,
            "owned_by": "llamacpp"
        }],
        "models": [{
            "name": "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M",
            "model": "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M",
            "meta": {"n_ctx": 262144}
        }]
    }"#;
    let response: ModelListResponse = serde_json::from_str(json).unwrap();
    assert_eq!(response.object, "list");
    // Same model ID in both arrays, so deduplicated to 1
    assert_eq!(response.data.len(), 1);
    let qwen_model = response.data.first().unwrap();
    assert_eq!(qwen_model.id, "Qwen/Qwen3-Coder-Next-GGUF:Q4_K_M");
    assert_eq!(qwen_model.max_model_len, Some(262144));
}
