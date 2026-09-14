/*
 * Class Name Utilities -- Maps Unity class IDs to readable class names.
 *
 * Follows Soul.md conventions: organized as struct + impl, no free functions.
 */

/**
 * Unity class name mapping utility class.
 *
 * Converts Unity class IDs to human-readable class name strings.
 * Does not involve any business logic; it is a pure utility method.
 *
 * Note: Some class IDs correspond to different type names in different Unity major versions,
 * so multiple IDs may map to the same string in the mapping table.
 * For example, 34 and 43 are both "Mesh" (ClassID registrations from different Unity versions),
 * both output "Mesh" when displayed to maintain frontend compatibility.
 */
pub struct ClassNameUtils;

impl ClassNameUtils {
    /**
     * Maps a Unity class ID to a class name.
     *
     * @param type_id Unity class ID
     * @return        Class name string, or None if the ID is unknown
     */
    pub fn get_class_name(type_id: i32) -> Option<&'static str> {
        Some(match type_id {
            1 => "GameObject",
            2 => "Component",
            3 => "LevelGameManager",
            4 => "Transform",
            5 => "TimeManager",
            6 => "GlobalGameManager",
            8 => "Behaviour",
            11 => "AudioManager",
            12 => "ParticleAnimator",
            13 => "InputManager",
            15 => "EllipsoidParticleEmitter",
            17 => "Pipeline",
            18 => "EditorExtension",
            19 => "Physics2DSettings",
            20 => "Camera",
            21 => "Material",
            23 => "MeshRenderer",
            25 => "Renderer",
            26 => "ParticleRenderer",
            27 => "Texture",
            28 => "Texture2D",
            29 => "OcclusionCullingSettings",
            30 => "GraphicsSettings",
            33 => "MeshFilter",
            34 => "Mesh",
            35 => "Skybox",
            36 => "QualitySettings",
            37 => "Shader",
            38 => "TextAsset",
            39 => "Rigidbody2D",
            40 => "Physics2DManager",
            41 => "OcclusionPortal",
            42 => "HingeJoint2D",
            43 => "Mesh",
            44 => "Rigidbody",
            45 => "SphereCollider",
            47 => "QualitySettings",
            48 => "Shader",
            49 => "TextAsset",
            50 => "Rigidbody2D",
            51 => "Physics2DManager",
            53 => "Collider2D",
            54 => "Rigidbody",
            55 => "PhysicsManager",
            56 => "Collider",
            57 => "Joint",
            58 => "CircleCollider2D",
            59 => "HingeJoint",
            60 => "PolygonCollider2D",
            61 => "BoxCollider2D",
            62 => "PhysicsMaterial2D",
            64 => "MeshCollider",
            65 => "BoxCollider",
            66 => "CompositeCollider2D",
            68 => "EdgeCollider2D",
            70 => "CapsuleCollider2D",
            72 => "ComputeShader",
            74 => "AnimationClip",
            75 => "ConstantForce",
            76 => "WorldParticleCollider",
            78 => "TagManager",
            81 => "AudioListener",
            82 => "AudioSource",
            83 => "AudioClip",
            84 => "RenderTexture",
            86 => "CustomRenderTexture",
            87 => "MeshParticleEmitter",
            88 => "ParticleEmitter",
            89 => "Cubemap",
            90 => "Avatar",
            91 => "AnimatorController",
            92 => "GUILayer",
            93 => "RuntimeAnimatorController",
            94 => "ShaderNameRegistry",
            95 => "Animator",
            96 => "TrailRenderer",
            98 => "DelayedCallManager",
            100 => "TextMesh",
            104 => "RenderSettings",
            108 => "Light",
            109 => "ShaderInclude",
            110 => "BaseAnimationTrack",
            111 => "Animation",
            114 => "MonoBehaviour",
            115 => "MonoScript",
            116 => "MonoManager",
            117 => "Texture3D",
            118 => "NewAnimationTrack",
            119 => "Projector",
            120 => "LineRenderer",
            121 => "Flare",
            122 => "Halo",
            123 => "LensFlare",
            124 => "FlareLayer",
            126 => "NavMeshProjectSettings",
            128 => "Font",
            129 => "PlayerSettings",
            130 => "NamedObject",
            134 => "PhysicsMaterial",
            135 => "SphereCollider",
            136 => "CapsuleCollider",
            137 => "SkinnedMeshRenderer",
            138 => "FixedJoint",
            141 => "BuildSettings",
            142 => "AssetBundle",
            143 => "CharacterController",
            144 => "CharacterJoint",
            145 => "SpringJoint",
            146 => "WheelCollider",
            147 => "ResourceManager",
            150 => "PreloadData",
            152 => "MovieTexture",
            153 => "ConfigurableJoint",
            154 => "TerrainCollider",
            156 => "TerrainData",
            157 => "LightmapSettings",
            158 => "WebCamTexture",
            159 => "EditorSettings",
            162 => "EditorUserSettings",
            164 => "AudioReverbFilter",
            165 => "AudioHighPassFilter",
            166 => "AudioChorusFilter",
            167 => "AudioReverbZone",
            168 => "AudioEchoFilter",
            169 => "AudioLowPassFilter",
            170 => "AudioDistortionFilter",
            171 => "SparseTexture",
            180 => "AudioBehaviour",
            181 => "AudioFilter",
            182 => "WindZone",
            183 => "Cloth",
            184 => "SubstanceArchive",
            185 => "ProceduralMaterial",
            186 => "ProceduralTexture",
            187 => "Texture2DArray",
            188 => "CubemapArray",
            191 => "OffMeshLink",
            192 => "OcclusionArea",
            193 => "Tree",
            195 => "NavMeshAgent",
            196 => "NavMeshSettings",
            198 => "ParticleSystem",
            199 => "ParticleSystemRenderer",
            200 => "ShaderVariantCollection",
            210 => "SortingGroup",
            212 => "SpriteRenderer",
            213 => "Sprite",
            215 => "ReflectionProbe",
            218 => "Terrain",
            220 => "LightProbeGroup",
            221 => "AnimatorOverrideController",
            222 => "CanvasRenderer",
            223 => "Canvas",
            224 => "RectTransform",
            225 => "CanvasGroup",
            241 => "AudioMixerController",
            243 => "AudioMixerGroupController",
            245 => "AudioMixerSnapshotController",
            258 => "LightProbes",
            271 => "Texture2D",
            290 => "AssetBundleManifest",
            319 => "AvatarMask",
            320 => "PlayableDirector",
            329 => "VideoClip",
            331 => "SpriteMask",
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::ClassNameUtils;

    #[test]
    fn audio_and_render_class_ids_map_to_authoritative_names() {
        assert_eq!(ClassNameUtils::get_class_name(81), Some("AudioListener"));
        assert_eq!(ClassNameUtils::get_class_name(82), Some("AudioSource"));
        assert_eq!(ClassNameUtils::get_class_name(83), Some("AudioClip"));
        assert_eq!(ClassNameUtils::get_class_name(84), Some("RenderTexture"));
    }

    #[test]
    fn mislabelled_class_ids_are_corrected() {
        // These were previously mapped to the wrong class names (cosmetic + routing).
        assert_eq!(ClassNameUtils::get_class_name(104), Some("RenderSettings"));
        assert_eq!(ClassNameUtils::get_class_name(119), Some("Projector"));
        assert_eq!(ClassNameUtils::get_class_name(135), Some("SphereCollider"));
        assert_eq!(ClassNameUtils::get_class_name(136), Some("CapsuleCollider"));
        assert_eq!(ClassNameUtils::get_class_name(182), Some("WindZone"));
        assert_eq!(ClassNameUtils::get_class_name(320), Some("PlayableDirector"));
        assert_eq!(ClassNameUtils::get_class_name(92), Some("GUILayer"));
        assert_eq!(ClassNameUtils::get_class_name(128), Some("Font"));
    }

    #[test]
    fn export_only_class_id_constants_are_authoritative() {
        // The exporters locate objects by class id; these must stay in sync with the mapping.
        assert_eq!(ClassNameUtils::get_class_name(49), Some("TextAsset"));
        assert_eq!(ClassNameUtils::get_class_name(83), Some("AudioClip"));
        assert_eq!(ClassNameUtils::get_class_name(128), Some("Font"));
    }
}
