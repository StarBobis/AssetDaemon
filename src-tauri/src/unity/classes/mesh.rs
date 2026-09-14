use crate::common::serialized_file::serialized_file::{ObjectInfo, SerializedFile};
use crate::unity::classes::object::UnityObjectHeader;
use crate::unity::mesh_asset_studio::{AssetStudioMesh, AssetStudioMeshParser};

#[allow(dead_code)]
pub struct MeshObject {
    pub header: UnityObjectHeader,
    pub data: AssetStudioMesh,
}

pub struct Mesh;

impl Mesh {
    pub fn read(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
        header: UnityObjectHeader,
    ) -> Result<MeshObject, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        let data = AssetStudioMeshParser::parse_mesh_from_raw(raw, &serialized_file.unity_version)?;
        Ok(MeshObject { header, data })
    }
}
