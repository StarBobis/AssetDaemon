use crate::common::serialized_file::object_reader::ObjectReader;
use crate::common::serialized_file::serialized_file::{ObjectInfo, SerializedFile};
use crate::unity::classes::registry::UnityClassParser;

pub struct UnityObjectNameUtils;

impl UnityObjectNameUtils {
    /// Borrowed object name reading without cloning SerializedFile.
    pub fn peek_object_name(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<Option<String>, String> {
        let raw = serialized_file.object_bytes(object_info)?;
        if raw.len() < 4 {
            return Ok(None);
        }
        let mut reader = ObjectReader::new(raw, serialized_file);
        let name = reader.read_aligned_string();
        if name.is_empty() {
            Ok(None)
        } else {
            Ok(Some(name))
        }
    }

    /// Unity Component assets such as Animator usually do not own `m_Name`.
    /// For display, AssetStudio-style browsers show the attached GameObject name.
    pub fn display_name(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<Option<String>, String> {
        if object_info.class_id == 1 {
            return Ok(
                UnityClassParser::parse_game_object(serialized_file, object_info)
                    .ok()
                    .map(|game_object| game_object.name)
                    .filter(|name| !name.is_empty()),
            );
        }

        if matches!(object_info.class_id, 95 | 111) {
            return Self::component_game_object_name(serialized_file, object_info);
        }

        if let Some(name) = Self::peek_object_name(serialized_file, object_info)? {
            return Ok(Some(name));
        }

        Ok(None)
    }

    fn component_game_object_name(
        serialized_file: &SerializedFile,
        object_info: &ObjectInfo,
    ) -> Result<Option<String>, String> {
        let game_object_path_id = match object_info.class_id {
            95 => UnityClassParser::parse_animator(serialized_file, object_info)
                .ok()
                .map(|animator| animator.component.game_object.path_id),
            111 => UnityClassParser::parse_animation(serialized_file, object_info)
                .ok()
                .map(|animation| animation.component.game_object.path_id),
            _ => None,
        }
        .filter(|path_id| *path_id != 0);

        let Some(game_object_path_id) = game_object_path_id else {
            return Ok(None);
        };
        let Some(game_object_info) = serialized_file
            .m_objects
            .iter()
            .find(|candidate| candidate.path_id == game_object_path_id && candidate.class_id == 1)
        else {
            return Ok(None);
        };

        Ok(
            UnityClassParser::parse_game_object(serialized_file, game_object_info)
                .ok()
                .map(|game_object| game_object.name)
                .filter(|name| !name.is_empty()),
        )
    }
}
