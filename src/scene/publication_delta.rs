//! Guarded publication of changed stable rows. No retained per-instance snapshot.
use super::*;
use bevy_render::scene_slots::SceneSlot;

#[derive(Clone, Default, PartialEq, Debug)]
pub(super) struct TransportCounts([usize; 32]);
impl TransportCounts {
    pub(super) fn add(&mut self, flags: u32) {
        for (bit, count) in self.0.iter_mut().enumerate() {
            *count += usize::from(flags & (1 << bit) != 0);
        }
    }
    fn remove(&mut self, flags: u32) -> Option<()> {
        for (bit, count) in self.0.iter_mut().enumerate() {
            *count = count.checked_sub(usize::from(flags & (1 << bit) != 0))?;
        }
        Some(())
    }
    fn flags(&self) -> u32 {
        self.0.iter().enumerate().fold(0, |flags, (bit, count)| {
            flags | if *count > 0 { 1 << bit } else { 0 }
        })
    }
}
#[derive(Default)]
pub(super) struct PreparedSources {
    // Delta removal counters are valid only if every prior input was published.
    pub(super) complete: bool,
    pub(super) meshes: HashMap<AssetId<Mesh>, GpuInstanceGeometryIds>,
    pub(super) vertex_buffers: HashMap<BufferId, u32>,
    pub(super) index_buffers: HashMap<BufferId, u32>,
    pub(super) materials: HashMap<AssetId<StandardMaterial>, u32, FixedHasher>,
    pub(super) transport: TransportCounts,
}
struct Row {
    slot: SceneSlot,
    geometry: GpuInstanceGeometryIds,
    material: u32,
    transform: Mat4,
    previous: Mat4,
}
struct Delta {
    rows: Vec<Row>,
    transport: TransportCounts,
}
fn simple_material(material: &GpuMaterial) -> bool {
    material.emissive == Vec3::ZERO
        && material.flags & (MATERIAL_FLAG_ALPHA_BLEND | MATERIAL_FLAG_DIFFUSE_BLEND) == 0
}
/// Complete validation precedes any TLAS, storage, membership or binding mutation.
fn preflight(
    cache: &SceneCache,
    dirty: &[u32],
    removed: &[u32],
    active: impl Fn(u32) -> bool,
    ready: impl Fn(AssetId<Mesh>) -> bool,
    geometry: impl Fn(AssetId<Mesh>) -> Option<GpuInstanceGeometryIds>,
) -> Option<Delta> {
    if !cache.prepared.complete {
        return None;
    }
    let storage = &cache.storage;
    let mut transport = cache.prepared.transport.clone();
    let mut visited: HashSet<u32> = HashSet::default();
    for &index in dirty.iter().chain(removed) {
        if !visited.insert(index) {
            return None;
        }
        if active(index) {
            let material = storage
                .materials
                .get()
                .get(*storage.material_ids.get().get(index as usize)? as usize)?;
            if !simple_material(material) {
                return None;
            }
            transport.remove(material.flags)?;
        }
    }
    let rows = dirty
        .iter()
        .map(|&index| {
            let input = cache.instance_inputs.get(index as usize)?.as_ref()?;
            let slot = cache.slots.get_part(input.entity)?;
            if slot.index != index || !ready(input.mesh) {
                return None;
            }
            let material = *cache.prepared.materials.get(&input.material)?;
            let gpu = storage.materials.get().get(material as usize)?;
            if !simple_material(gpu) {
                return None;
            }
            transport.add(gpu.flags);
            Some(Row {
                slot,
                material,
                geometry: geometry(input.mesh)?,
                transform: Mat4::from(input.transform),
                previous: Mat4::from(input.previous),
            })
        })
        .collect::<Option<Vec<_>>>()?;
    // A last instance can remove a transport capability from the sky parameters.
    // Rebuild those bindings through the full path rather than retaining stale flags.
    if transport.flags() != cache.prepared.transport.flags() {
        return None;
    }
    Some(Delta { rows, transport })
}
pub(super) fn try_publish(
    cache: &mut SceneCache,
    dirty: &[u32],
    removed: &[u32],
    blas: &BlasManager,
    allocator: Option<&MeshAllocator>,
    device: &RenderDevice,
    queue: &RenderQueue,
    diagnostics: Option<&bevy_render::diagnostic::DiagnosticsRecorder>,
    bindings: &mut RaytracingSceneBindings,
    capture_indices: bool,
) -> bool {
    let capacity = cache.slots.capacity();
    let required = cache.slots.counts().0;
    let Some(tlas) = cache.tlas.get() else {
        return false;
    };
    if capacity != cache.storage.transforms.get().len()
        || tlas.get().len() < required
        || required == 0
    {
        return false;
    }
    // A delta is allowed only when every old live input was ready. Retirement
    // has already removed old owners from SceneSlots, but their published
    // material records remain available until this complete preflight succeeds.
    let preflight_profile = bevy_render::diagnostic::profile_scope("scene.delta_preflight");
    let removed_set: HashSet<u32> = removed.iter().copied().collect();
    let refreshed = if let Some(allocator) = allocator {
        let dirty_meshes: HashSet<_> = dirty
            .iter()
            .filter_map(|index| {
                cache
                    .instance_inputs
                    .get(*index as usize)?
                    .as_ref()
                    .map(|input| input.mesh)
            })
            .collect();
        let mut refreshed = HashMap::default();
        for (&mesh, old) in &cache.prepared.meshes {
            let Some(vertex) = allocator.mesh_vertex_slice(&mesh) else {
                return false;
            };
            let Some(index) = allocator.mesh_index_slice(&mesh) else {
                return false;
            };
            let Some(&vertex_buffer_id) = cache.prepared.vertex_buffers.get(&vertex.buffer.id())
            else {
                return false;
            };
            let Some(&index_buffer_id) = cache.prepared.index_buffers.get(&index.buffer.id())
            else {
                return false;
            };
            let next = GpuInstanceGeometryIds {
                vertex_buffer_id,
                vertex_buffer_offset: vertex.range.start,
                index_buffer_id,
                index_buffer_offset: index.range.start,
                triangle_count: (index.range.len() / 3) as u32,
                light_probability: old.light_probability,
            };
            // Relocation outside the dirty set needs the full publication.
            if next != *old && !dirty_meshes.contains(&mesh) {
                return false;
            }
            refreshed.insert(mesh, next);
        }
        Some(refreshed)
    } else {
        None
    };
    let Some(delta) = preflight(
        cache,
        dirty,
        removed,
        |index| {
            removed_set.contains(&index)
                || cache
                    .instance_inputs
                    .get(index as usize)
                    .and_then(Option::as_ref)
                    .and_then(|input| cache.slots.get_part(input.entity))
                    .is_some_and(|slot| cache.slots.is_active(slot))
        },
        |mesh| blas.get(&mesh).is_some(),
        |mesh| {
            refreshed
                .as_ref()
                .unwrap_or(&cache.prepared.meshes)
                .get(&mesh)
                .cloned()
        },
    ) else {
        return false;
    };
    drop(preflight_profile);
    let descriptor_profile = bevy_render::diagnostic::profile_scope("scene.delta_descriptors");
    let dirty_set = super::super::tlas::DirtySlots::new(capacity, dirty);
    let next = cache.slots.active_indices().iter().copied().chain(
        delta
            .rows
            .iter()
            .filter(|row| !cache.slots.is_active(row.slot))
            .map(|row| row.slot.index),
    );
    let mut next_count = 0;
    let changed = super::super::tlas::changed_dense_slots(
        cache.storage.active_indices.get(),
        next.inspect(|_| next_count += 1),
        |slot| dirty_set.contains(slot),
    );
    if next_count != required {
        return false;
    }
    // Validate displaced survivors as well as explicit dirty rows before any
    // ownership, storage or descriptor mutation. Only changed descriptors are held.
    let updates = changed
        .into_iter()
        .map(|(address, index)| {
            let input = cache.instance_inputs.get(index as usize)?.as_ref()?;
            let slot = cache.slots.get_part(input.entity)?;
            if slot.index != index {
                return None;
            }
            Some((
                address,
                TlasInstance::new(
                    blas.get(&input.mesh)?,
                    tlas_transform(&Mat4::from(input.transform)),
                    index,
                    0xFF,
                ),
            ))
        })
        .collect::<Option<Vec<_>>>();
    let Some(updates) = updates else {
        return false;
    };
    drop(descriptor_profile);
    let _profile = bevy_render::diagnostic::profile_scope("scene.prepare_instances");
    let SceneCache {
        storage,
        slots,
        tlas,
        prepared,
        ..
    } = cache;
    let tlas = tlas.retained(required).expect("preflighted TLAS");
    let old_count = storage.active_indices.get().len();
    bevy_render::diagnostic::profile_value(
        "scene.tlas_publication_rows",
        (updates.len() + old_count.saturating_sub(required)) as f64,
        "count",
    );
    for index in required..old_count {
        super::super::tlas::publish_slot(tlas, index as u32, None);
    }
    for (address, instance) in updates {
        super::super::tlas::publish_slot(tlas, address, Some(instance));
    }
    for row in delta.rows {
        let index = row.slot.index as usize;
        storage.transforms.get_mut()[index] = row.transform;
        storage.previous_frame_transforms.get_mut()[index] = row.previous;
        storage.geometry_ids.get_mut()[index] = row.geometry;
        storage.material_ids.get_mut()[index] = row.material;
        slots.activate(row.slot);
    }
    prepared.transport = delta.transport;
    if let Some(refreshed) = refreshed {
        prepared.meshes = refreshed;
    }
    let active_dirty = storage.active_indices.set_indices(slots.active_indices());
    bevy_render::diagnostic::profile_value("scene.delta_publication", 1.0, "count");
    bevy_render::diagnostic::profile_value(
        "scene.publication_rows",
        (dirty.len() + removed.len()) as f64,
        "count",
    );
    bevy_render::diagnostic::profile_value("scene.tlas_allocation_reused", 1.0, "count");
    drop(_profile);
    upload(storage, dirty, &active_dirty, device, queue);
    #[cfg(feature = "graphics_debug")]
    {
        // All scene-bound allocations are unchanged. The diagnostic-only active
        // list may grow independently, and needs a fresh readback handle.
        bindings
            .debug
            .buffers
            .retain(|(role, _)| *role != "ray_active_indices");
        if let Some(buffer) = storage.active_indices.buffer() {
            bindings
                .debug
                .buffers
                .push(("ray_active_indices", buffer.clone()));
        }
        bindings.debug.instances = slots.counts().0;
        bindings.debug.tlas_active = slots.active_indices().len();
        bindings.debug.tlas_capacity = tlas.get().len();
        bindings.debug.active_indices = if capture_indices {
            slots.active_indices().to_vec()
        } else {
            Vec::new()
        };
    }
    #[cfg(not(feature = "graphics_debug"))]
    let _ = (bindings, capture_indices);
    let mut encoder = device.create_command_encoder(&CommandEncoderDescriptor {
        label: Some("build_tlas_command_encoder"),
    });
    use bevy_render::diagnostic::RecordDiagnostics;
    let span = diagnostics.time_span(&mut encoder, "scene/tlas_build");
    encoder.build_acceleration_structures(&[], [&*tlas]);
    span.end(&mut encoder);
    queue.submit([encoder.finish()]);
    true
}
fn upload(
    storage: &mut SceneStorage,
    dirty: &[u32],
    active: &[u32],
    device: &RenderDevice,
    queue: &RenderQueue,
) {
    let _profile = bevy_render::diagnostic::profile_scope("scene.upload");
    let mut batch = StorageBufferUploadBatch::default();
    let uploads = [
        storage
            .transforms
            .stage_buffer_indices(device, queue, dirty, &mut batch),
        storage
            .previous_frame_transforms
            .stage_buffer_indices(device, queue, dirty, &mut batch),
        storage
            .geometry_ids
            .stage_buffer_indices(device, queue, dirty, &mut batch),
        storage
            .material_ids
            .stage_buffer_indices(device, queue, dirty, &mut batch),
        storage
            .active_indices
            .stage_buffer_indices(device, queue, active, &mut batch),
    ];
    let (bytes, buffers) = batch.finish(device, queue);
    for (name, value, unit) in [
        ("scene.staging_bytes", bytes as f64, "bytes"),
        ("scene.staging_buffers", buffers as f64, "count"),
        (
            "scene.upload_bytes",
            uploads.iter().map(|u| u.bytes as f64).sum(),
            "bytes",
        ),
        (
            "scene.upload_ranges",
            uploads.iter().map(|u| f64::from(u.ranges)).sum(),
            "count",
        ),
        (
            "scene.buffer_allocations",
            uploads.iter().filter(|u| u.allocated).count() as f64,
            "count",
        ),
    ] {
        bevy_render::diagnostic::profile_value(name, value, unit);
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn preflight(
        cache: &SceneCache,
        dirty: &[u32],
        removed: &[u32],
        active: impl Fn(u32) -> bool,
        ready: impl Fn(AssetId<Mesh>) -> bool,
    ) -> Option<Delta> {
        super::preflight(cache, dirty, removed, active, ready, |mesh| {
            cache.prepared.meshes.get(&mesh).cloned()
        })
    }

    #[test]
    fn refreshed_mesh_rows_are_published_only_after_complete_preflight() {
        let (cache, dirty) = fixture();
        let replacement = GpuInstanceGeometryIds {
            vertex_buffer_offset: 128,
            index_buffer_offset: 64,
            triangle_count: 9,
            ..Default::default()
        };
        let delta = super::preflight(
            &cache,
            &dirty,
            &[],
            |_| true,
            |_| true,
            |_| Some(replacement.clone()),
        )
        .unwrap();
        assert!(delta.rows.iter().all(|row| row.geometry == replacement));
        assert!(super::preflight(&cache, &dirty, &[], |_| true, |_| true, |_| None).is_none());
        assert_eq!(
            cache
                .prepared
                .meshes
                .values()
                .next()
                .unwrap()
                .triangle_count,
            3
        );
    }

    fn fixture() -> (SceneCache, Vec<u32>) {
        let mut cache = SceneCache::default();
        let mut dirty = Vec::new();
        cache.slots.begin();
        for id in 1..=3 {
            observe_instance(
                &mut cache,
                RayInstanceInput {
                    entity: Entity::from_bits(id).into(),
                    mesh: Handle::<Mesh>::default().id(),
                    material: Handle::<StandardMaterial>::default().id(),
                    transform: Affine3A::from_translation(Vec3::X * id as f32),
                    previous: Affine3A::IDENTITY,
                },
                false,
                &mut dirty,
            );
        }
        cache.storage.materials.get_mut().push(GpuMaterial {
            normal_map_texture_id: TEXTURE_MAP_NONE,
            base_color_texture_id: TEXTURE_MAP_NONE,
            emissive_texture_id: TEXTURE_MAP_NONE,
            metallic_roughness_texture_id: TEXTURE_MAP_NONE,
            base_color: Vec3::ONE,
            perceptual_roughness: 1.0,
            emissive: Vec3::ZERO,
            metallic: 0.0,
            alpha_cutoff: 0.5,
            flags: MATERIAL_FLAG_ALPHA_MASK | MATERIAL_FLAG_DOUBLE_SIDED,
            base_color_alpha: 1.0,
            reflectance: 0.5,
            emission_cone: Vec4::ZERO,
            gaussian: Vec4::ZERO,
        });
        cache
            .storage
            .material_ids
            .get_mut()
            .resize(cache.slots.capacity(), 0);
        cache
            .prepared
            .materials
            .insert(Handle::<StandardMaterial>::default().id(), 0);
        cache.prepared.meshes.insert(
            Handle::<Mesh>::default().id(),
            GpuInstanceGeometryIds {
                triangle_count: 3,
                ..Default::default()
            },
        );
        for _ in &dirty {
            cache
                .prepared
                .transport
                .add(cache.storage.materials.get()[0].flags);
        }
        cache.prepared.complete = true;
        (cache, dirty)
    }

    #[test]
    fn delta_rows_equal_full_rows_under_reorder_motion_and_return() {
        let (mut cache, dirty) = fixture();
        for x in [100.0, -20.0, 1.0] {
            let input = cache.instance_inputs[dirty[0] as usize].as_mut().unwrap();
            input.previous = input.transform;
            input.transform = Affine3A::from_translation(Vec3::X * x);
            let delta = preflight(&cache, &[dirty[2], dirty[0]], &[], |_| true, |_| true).unwrap();
            for row in delta.rows {
                let full = cache.instance_inputs[row.slot.index as usize]
                    .as_ref()
                    .unwrap();
                assert_eq!(row.transform, Mat4::from(full.transform));
                assert_eq!(row.previous, Mat4::from(full.previous));
                assert_eq!(row.material, cache.prepared.materials[&full.material]);
                assert!(row.geometry == cache.prepared.meshes[&full.mesh]);
                assert_eq!(row.slot, cache.slots.get_part(full.entity).unwrap());
            }
            assert_eq!(delta.transport, cache.prepared.transport);
        }
    }

    #[test]
    fn removal_and_new_generation_do_not_retain_transport_or_owner() {
        let (mut cache, dirty) = fixture();
        let removed = dirty[0];
        cache.instance_inputs[removed as usize] = None;
        let delta = preflight(&cache, &[dirty[1]], &[removed], |_| true, |_| true).unwrap();
        assert_eq!(delta.rows.len(), 1);
        let mut expected = cache.prepared.transport.clone();
        expected
            .remove(cache.storage.materials.get()[0].flags)
            .unwrap();
        assert_eq!(delta.transport, expected);
        assert!(
            preflight(&cache, &[], &dirty, |_| true, |_| true).is_none(),
            "empty transport uses the full empty publication"
        );
        let mut other_owner = cache.instance_inputs[dirty[1] as usize].clone().unwrap();
        other_owner.entity = Entity::from_bits(99).into();
        cache.instance_inputs[dirty[1] as usize] = Some(other_owner);
        assert!(
            preflight(&cache, &[dirty[1]], &[], |_| true, |_| true).is_none(),
            "a recycled address cannot validate an unrelated owner"
        );
    }

    #[test]
    fn all_failure_conditions_leave_prepared_state_unchanged() {
        let (mut cache, dirty) = fixture();
        let counts = cache.prepared.transport.clone();
        let before = cache.instance_inputs.clone();
        cache.prepared.complete = false;
        assert!(preflight(&cache, &dirty, &[], |_| true, |_| true).is_none());
        cache.prepared.complete = true;
        assert!(preflight(&cache, &dirty, &[], |_| true, |_| false).is_none());
        assert!(preflight(&cache, &[dirty[0], dirty[0]], &[], |_| true, |_| true).is_none());
        let geometry = cache.prepared.meshes.drain().next().unwrap();
        assert!(preflight(&cache, &dirty, &[], |_| true, |_| true).is_none());
        cache.prepared.meshes.insert(geometry.0, geometry.1);
        let material = cache.prepared.materials.drain().next().unwrap();
        assert!(preflight(&cache, &dirty, &[], |_| true, |_| true).is_none());
        cache.prepared.materials.insert(material.0, material.1);
        cache.storage.materials.get_mut()[0].emissive = Vec3::ONE;
        assert!(preflight(&cache, &dirty, &[], |_| true, |_| true).is_none());
        cache.storage.materials.get_mut()[0].emissive = Vec3::ZERO;
        cache.storage.materials.get_mut()[0].flags |= MATERIAL_FLAG_ALPHA_BLEND;
        assert!(preflight(&cache, &dirty, &[], |_| true, |_| true).is_none());
        assert_eq!(cache.prepared.transport, counts);
        assert!(cache.instance_inputs == before);
    }
}
