#version 330
in vec2 fragTexCoord;
in vec4 fragColor;
in vec3 fragWorldPos;
out vec4 finalColor;
uniform sampler2D texture0;
uniform vec3 fogColor;
uniform float fogStart;
uniform float fogEnd;
uniform vec3 cameraPos;
// Underwater enhancements
uniform float time;
uniform int underwater;
void main(){
  // Subtle UV warp when underwater to simulate refractive wobble
  vec2 uv = fragTexCoord;
  if (underwater > 0) {
    float w = sin(fragWorldPos.x * 0.13 + time * 0.8) * 0.008 + cos(fragWorldPos.z * 0.17 - time * 0.6) * 0.008;
    uv += vec2(w, w);
  }
  vec4 base = texture(texture0, uv) * fragColor;
  // Simple linear fog based on world-space distance from camera
  float dist = length(fragWorldPos - cameraPos);
  float f = clamp((fogEnd - dist) / max(fogEnd - fogStart, 0.0001), 0.0, 1.0);
  vec3 rgb = mix(fogColor, base.rgb, f);
  // Extra tint when underwater
  if (underwater > 0) {
    rgb = mix(fogColor, rgb, 0.85);
  }
  finalColor = vec4(rgb, base.a);
}
