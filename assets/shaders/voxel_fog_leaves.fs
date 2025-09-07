#version 330
in vec2 fragTexCoord;
in vec4 fragColor;
in vec3 fragWorldPos;
out vec4 finalColor;
uniform sampler2D texture0;
// Fog uniforms (match voxel_fog_textured)
uniform vec3 fogColor;
uniform float fogStart;
uniform float fogEnd;
uniform vec3 cameraPos;
// Cutout threshold: discard fully transparent texels to avoid depth holes
uniform float alphaCutoff;
// Autumn palette uniforms
uniform vec3 palette0; // low -> high stops across grayscale
uniform vec3 palette1;
uniform vec3 palette2;
uniform vec3 palette3;
uniform float autumnStrength; // 0..1: blend between grayscale and palette

vec3 gradientMap(float t){
  t = clamp(t, 0.0, 1.0);
  if (t < 1.0/3.0) {
    float u = t * 3.0;
    return mix(palette0, palette1, u);
  } else if (t < 2.0/3.0) {
    float u = (t - 1.0/3.0) * 3.0;
    return mix(palette1, palette2, u);
  } else {
    float u = (t - 2.0/3.0) * 3.0;
    return mix(palette2, palette3, u);
  }
}

void main(){
  vec4 tex = texture(texture0, fragTexCoord);
  // Alpha cutout: prevent transparent pixels from writing depth
  if (tex.a < alphaCutoff) discard;
  // Grayscale intensity from the leaves texture
  float g = dot(tex.rgb, vec3(0.299, 0.587, 0.114));
  vec3 autumn = gradientMap(g);
  // Blend with original grayscale to control intensity
  vec3 base = mix(vec3(g), autumn, clamp(autumnStrength, 0.0, 1.0));
  // Apply per-vertex brightness (AO/lighting) via fragColor.rgb
  base *= fragColor.rgb;
  // Linear fog based on distance
  float dist = length(fragWorldPos - cameraPos);
  float f = clamp((fogEnd - dist) / max(fogEnd - fogStart, 0.0001), 0.0, 1.0);
  vec3 rgb = mix(fogColor, base, f);
  finalColor = vec4(rgb, tex.a); // use texture alpha (opaque leaves textures)
}
