use amethystate::ReactiveMap;
use amethystate::migration::types::AmeType;

#[amethystate::amethystate]
pub struct DeepChildV1 {
    #[amestate(default = 0)]
    pub count: u32,
}

#[amethystate::amethystate]
pub struct DeepChildV2WithRenamedField {
    #[amestate(default = 0)]
    pub counter: u32,
}

#[amethystate::amethystate]
pub struct MidParentV1 {
    #[amestate(nested)]
    pub leaf: DeepChildV1,
}

#[amethystate::amethystate]
pub struct MidParentV2 {
    #[amestate(nested)]
    pub leaf: DeepChildV2WithRenamedField,
}

#[amethystate::amethystate(prefix = "app")]
pub struct RootV1 {
    #[amestate(nested)]
    pub mid: MidParentV1,
}

#[amethystate::amethystate(prefix = "app")]
pub struct RootV2WithDeepChange {
    #[amestate(nested)]
    pub mid: MidParentV2,
}

#[amethystate::amethystate(prefix = "network")]
pub struct NetworkConfigV1 {
    pub dns_servers: ReactiveMap<String, String>,
}

#[amethystate::amethystate(prefix = "network")]
pub struct NetworkConfigV2WithDifferentKey {
    pub dns_servers: ReactiveMap<u32, String>,
}

#[amethystate::amethystate(prefix = "network")]
pub struct NetworkConfigV2WithDifferentValue {
    pub dns_servers: ReactiveMap<String, u64>,
}

#[amethystate::amethystate(prefix = "service")]
pub struct ServiceV1 {
    #[amestate(default = false, volatile)]
    pub is_dirty: bool,

    #[amestate(default = 10)]
    pub max_connections: u32,
}

#[amethystate::amethystate(prefix = "service")]
pub struct ServiceV2WithChangedVolatile {
    #[amestate(default = 0, volatile)]
    pub is_dirty: u8,

    #[amestate(default = 10)]
    pub max_connections: u32,
}

const _: () = {
    assert!(
        DeepChildV1_Data::TYPE_HASH != DeepChildV2WithRenamedField_Data::TYPE_HASH,
        "Deep child change must update its own TYPE_HASH"
    );

    assert!(
        MidParentV1_Data::TYPE_HASH != MidParentV2_Data::TYPE_HASH,
        "Deep child change must propagate to MidParent's TYPE_HASH"
    );

    assert!(
        RootV1_Data::TYPE_HASH != RootV2WithDeepChange_Data::TYPE_HASH,
        "Deep child change must propagate all the way up to Root's TYPE_HASH"
    );

    assert!(
        NetworkConfigV1_Data::TYPE_HASH != NetworkConfigV2WithDifferentKey_Data::TYPE_HASH,
        "ReactiveMap key type change must affect TYPE_HASH"
    );
    assert!(
        NetworkConfigV1_Data::TYPE_HASH != NetworkConfigV2WithDifferentValue_Data::TYPE_HASH,
        "ReactiveMap value type change must affect TYPE_HASH"
    );

    assert!(
        ServiceV1_Data::TYPE_HASH == ServiceV2WithChangedVolatile_Data::TYPE_HASH,
        "A volatile field is not stored, so changing it must not affect _Data TYPE_HASH"
    );
};
