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
uniform float time;
uniform int underwater;
void main(){
  // Subtle UV distortion based on world position and time
  float wave = sin(fragWorldPos.x * 0.15 + time * 0.8) * 0.01 + cos(fragWorldPos.z * 0.12 - time * 0.6) * 0.01;
  vec2 uv = fragTexCoord + vec2(wave, wave);
  vec4 base = texture(texture0, uv) * fragColor;
  // Alpha depends on whether the camera is underwater
  // When underwater, make the surface opaque so nothing above is visible
  float alpha = (underwater > 0) ? 1.0 : 0.7;
  // Fog
  float dist = length(fragWorldPos - cameraPos);
  float f = clamp((fogEnd - dist) / max(fogEnd - fogStart, 0.0001), 0.0, 1.0);
  vec3 rgb = mix(fogColor, base.rgb, f);
  // Stronger tint when underwater, still draw surface from below (requires backface disabled)
  if (underwater > 0) {
    rgb = mix(fogColor, rgb, 0.70);
  }
  finalColor = vec4(rgb, alpha);
}
