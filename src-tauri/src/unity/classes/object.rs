#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct UnityObjectHeader {
    pub path_id: i64,
    pub class_id: i32,
    pub class_name: String,
    pub byte_size: u32,
    pub unity_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PPtr {
    pub file_id: i32,
    pub path_id: i64,
}

impl PPtr {
    pub fn is_null(&self) -> bool {
        self.path_id == 0 || self.file_id < 0
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RawUnityObject {
    pub header: UnityObjectHeader,
    pub bytes: Vec<u8>,
}
