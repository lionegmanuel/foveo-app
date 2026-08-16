//! Test de integracion real: crea un `Compositor` de verdad (adaptador +
//! device de GPU reales de esta maquina) y verifica que crop+scale produce
//! el resultado esperado sobre frames sinteticos conocidos. No esta
//! `#[ignore]`d porque no tiene side effects fuera del proceso (a diferencia
//! de `capture`/`input-tracker`, no toca pantalla ni mouse real) — corre en
//! cada `cargo test` normal.

use compositor::Compositor;
use project::Rect;

fn solid_frame(width: u32, height: u32, bgra: [u8; 4]) -> Vec<u8> {
    let mut buf = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..(width * height) {
        buf.extend_from_slice(&bgra);
    }
    buf
}

#[test]
fn full_frame_crop_reproduces_a_solid_color() {
    let mut compositor = Compositor::new(64, 64, 32, 32).expect("no se pudo crear el Compositor (GPU)");
    let input = solid_frame(64, 64, [10, 20, 30, 255]); // B, G, R, A

    let output = compositor.composite_frame(&input, Rect::FULL_FRAME).expect("composite_frame fallo");

    assert_eq!(output.len(), compositor.output_frame_size());
    assert_eq!(output.len(), 32 * 32 * 4);

    // Cada pixel deberia seguir siendo el mismo color solido (con margen de
    // error chico por el filtro lineal del sampler, que en un frame de color
    // uniforme no deberia moverse practicamente nada).
    for chunk in output.chunks_exact(4) {
        assert!((chunk[0] as i32 - 10).abs() <= 2, "B fuera de rango: {chunk:?}");
        assert!((chunk[1] as i32 - 20).abs() <= 2, "G fuera de rango: {chunk:?}");
        assert!((chunk[2] as i32 - 30).abs() <= 2, "R fuera de rango: {chunk:?}");
        assert_eq!(chunk[3], 255);
    }
}

#[test]
fn cropping_to_left_half_only_shows_the_left_halfs_color() {
    let width = 64u32;
    let height = 64u32;
    let mut input = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            let _ = y;
            if x < width / 2 {
                input.extend_from_slice(&[200, 0, 0, 255]); // izquierda: azul fuerte (BGRA)
            } else {
                input.extend_from_slice(&[0, 0, 200, 255]); // derecha: rojo fuerte (BGRA)
            }
        }
    }

    let mut compositor = Compositor::new(width, height, 16, 16).expect("no se pudo crear el Compositor (GPU)");

    // Crop a la mitad izquierda exacta.
    let left_half = Rect { x: 0.0, y: 0.0, w: 0.5, h: 1.0 };
    let output = compositor.composite_frame(&input, left_half).expect("composite_frame fallo");

    for chunk in output.chunks_exact(4) {
        assert!(chunk[0] > 150, "deberia quedar el azul de la mitad izquierda: {chunk:?}");
        assert!(chunk[2] < 50, "no deberia verse nada de la mitad derecha: {chunk:?}");
    }
}

#[test]
fn rejects_input_buffer_with_wrong_size() {
    let mut compositor = Compositor::new(64, 64, 32, 32).expect("no se pudo crear el Compositor (GPU)");
    let bad_input = vec![0u8; 10];

    let err = compositor.composite_frame(&bad_input, Rect::FULL_FRAME).unwrap_err();
    assert!(matches!(err, compositor::CompositorError::BadInputSize { .. }));
}
