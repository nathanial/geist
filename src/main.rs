use raylib::prelude::*;

fn main() {
    // Create a window and Raylib handle
    let (mut rl, thread) = raylib::init()
        .size(800, 450)
        .title("Geist + Raylib: Simple Scene")
        .build();

    // Basic 3D camera looking at the origin
    let mut camera = Camera3D::perspective(
        Vector3::new(4.0, 4.0, 4.0),
        Vector3::new(0.0, 1.0, 0.0),
        Vector3::new(0.0, 1.0, 0.0),
        45.0,
    );

    rl.set_target_fps(60);

    while !rl.window_should_close() {
        // Draw 3D and 2D overlays
        let mut d = rl.begin_drawing(&thread);
        d.clear_background(Color::RAYWHITE);

        {
            let mut d3 = d.begin_mode3D(camera);
            // Ground grid
            d3.draw_grid(20, 1.0);
            // A cube at the origin
            d3.draw_cube(Vector3::new(0.0, 0.5, 0.0), 1.0, 1.0, 1.0, Color::SKYBLUE);
            d3.draw_cube_wires(Vector3::new(0.0, 0.5, 0.0), 1.0, 1.0, 1.0, Color::BLUE);
        }

        d.draw_text("Hello from Raylib!", 12, 12, 20, Color::DARKGRAY);
        d.draw_fps(12, 40);
    }
}
