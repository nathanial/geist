#version 330
in vec2 fragTexCoord;
in vec4 fragColor;
in vec3 fragWorldPos;
out vec4 finalColor;
uniform sampler2D texture0;
uniform float alphaCutoff;
uniform vec3 fogColor;
uniform float fogStart;
uniform float fogEnd;
uniform vec3 cameraPos;
void main(){
  vec4 tex = texture(texture0, fragTexCoord);
  if (tex.a < alphaCutoff) discard;
  vec4 base = vec4(tex.rgb, tex.a) * fragColor;
  float dist = length(fragWorldPos - cameraPos);
  float f = clamp((fogEnd - dist) / max(fogEnd - fogStart, 0.0001), 0.0, 1.0);
  vec3 rgb = mix(fogColor, base.rgb, f);
  finalColor = vec4(rgb, base.a);
}

