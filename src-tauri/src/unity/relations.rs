pub struct UnityRelationKind;

impl UnityRelationKind {
    // ===== 网格/渲染器 =====
    pub const RENDERER_MESH: &'static str = "renderer_mesh";
    pub const MESH_FILTER_MESH: &'static str = "mesh_filter_mesh";
    pub const MESH_FILTER_GAMEOBJECT: &'static str = "mesh_filter_gameobject";
    pub const RENDERER_GAMEOBJECT: &'static str = "renderer_gameobject";
    pub const RENDERER_MATERIAL: &'static str = "renderer_material";
    pub const SPRITE_MASK_GAMEOBJECT: &'static str = "sprite_mask_gameobject";
    pub const SPRITE_MASK_SPRITE: &'static str = "sprite_mask_sprite";

    // ===== 动画链路（Mecanim Animator）=====
    /// Animator → 挂载的 GameObject
    pub const ANIMATOR_GAMEOBJECT: &'static str = "animator_gameobject";
    /// Animator → Avatar
    pub const ANIMATOR_AVATAR: &'static str = "animator_avatar";
    /// Animator → RuntimeAnimatorController / AnimatorController
    pub const ANIMATOR_CONTROLLER: &'static str = "animator_controller";
    /// AnimatorController → AnimationClip
    pub const CONTROLLER_ANIMATION_CLIP: &'static str = "controller_animation_clip";
    /// AnimatorOverrideController → base AnimatorController
    pub const OVERRIDE_BASE_CONTROLLER: &'static str = "override_base_controller";
    /// AnimatorOverrideController → m_Clips[i].m_OriginalClip
    pub const OVERRIDE_CLIP_ORIGINAL: &'static str = "override_clip_original";
    /// AnimatorOverrideController → m_Clips[i].m_OverrideClip
    pub const OVERRIDE_CLIP_OVERRIDE: &'static str = "override_clip_override";

    // ===== 动画链路（Legacy Animation）=====
    /// Animation → 挂载的 GameObject
    pub const ANIMATION_GAMEOBJECT: &'static str = "animation_gameobject";
    /// Animation → default AnimationClip (m_Animation)
    pub const ANIMATION_DEFAULT_CLIP: &'static str = "animation_default_clip";
    /// Animation → m_Animations[]
    pub const ANIMATION_CLIPS: &'static str = "animation_clips";
}
