#version 330
in vec2 fragTexCoord;
in vec4 fragColor;
in vec3 fragWorldPos;
in vec3 fragNormal;
out vec4 finalColor;
uniform sampler2D texture0;
// Phase 2 lighting
uniform sampler2D lightTex;
uniform ivec3 lightDims;            // (sx+2, sy+2, sz+2) including seam rings
uniform ivec2 lightGrid;
uniform vec3  chunkOrigin;
uniform float visualLightMin;
uniform float skyLightScale;
// Fog uniforms (match voxel_fog_textured)
uniform vec3 fogColor;
uniform float fogStart;
uniform float fogEnd;
uniform vec3 cameraPos;
uniform float time;
uniform int underwater;
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

vec2 lightAtlasUV(ivec3 v) {
  int tile_w = lightDims.x;
  int tile_h = lightDims.z;
  int cols = max(lightGrid.x, 1);
  int tx = v.y % cols;
  int ty = v.y / cols;
  int px = tx * tile_w + v.x;
  int py = ty * tile_h + v.z;
  float u = (float(px) + 0.5) / float(tile_w * cols);
  float vuv = (float(py) + 0.5) / float(tile_h * max(lightGrid.y, 1));
  return vec2(u, vuv);
}

float sampleBrightness(vec3 worldPos, vec3 nrm) {
  // If lighting uniforms are unset for this draw, avoid sampling a stale texture
  if (lightDims.x == 0 || lightDims.y == 0 || lightDims.z == 0) {
    return visualLightMin;
  }
  // Interior dims exclude seam rings on all axes
  vec3 p = worldPos - chunkOrigin;
  ivec3 innerDims = ivec3(lightDims.x - 2, lightDims.y - 2, lightDims.z - 2);
  ivec3 vInner = ivec3(clamp(floor(p), vec3(0.0), vec3(innerDims) - vec3(1.0)));
  ivec3 step = ivec3(0,0,0);
  if (abs(nrm.x) > abs(nrm.y) && abs(nrm.x) > abs(nrm.z)) {
    step.x = (nrm.x > 0.0) ? 1 : -1;
  } else if (abs(nrm.z) > abs(nrm.y)) {
    step.z = (nrm.z > 0.0) ? 1 : -1;
  } else {
    step.y = (nrm.y > 0.0) ? 1 : -1;
  }
  ivec3 vnInner = vInner + step;
  vnInner.x = clamp(vnInner.x, -1, innerDims.x);
  vnInner.z = clamp(vnInner.z, -1, innerDims.z);
  vnInner.y = clamp(vnInner.y, -1, innerDims.y);
  ivec3 vAtlas = vInner + ivec3(1, 1, 1);
  ivec3 vnAtlas = vnInner + ivec3(1, 1, 1);
  vec2 uv0 = lightAtlasUV(vAtlas);
  vec3 l0 = texture(lightTex, uv0).rgb;
  vec2 uv1 = lightAtlasUV(vnAtlas);
  vec3 l1 = texture(lightTex, uv1).rgb;
  float blk = max(l0.r, l1.r);
  float sky = max(l0.g, l1.g) * clamp(skyLightScale, 0.0, 1.0);
  float bcn = max(l0.b, l1.b);
  float lv = max(blk, max(sky, bcn));
  return max(lv, visualLightMin);
}

void main(){
  vec2 uv = fragTexCoord;
  if (underwater > 0) {
    float w = sin(fragWorldPos.x * 0.13 + time * 0.8) * 0.008 + cos(fragWorldPos.z * 0.17 - time * 0.6) * 0.008;
    uv += vec2(w, w);
  }
  vec4 tex = texture(texture0, uv);
  // Grayscale intensity from the leaves texture
  float g = dot(tex.rgb, vec3(0.299, 0.587, 0.114));
  vec3 autumn = gradientMap(g);
  // Blend with original grayscale to control intensity
  vec3 base = mix(vec3(g), autumn, clamp(autumnStrength, 0.0, 1.0));
  // Apply per-vertex brightness (AO/lighting) via fragColor.rgb
  base *= fragColor.rgb;
  // Shader-sampled light
  float bright = sampleBrightness(fragWorldPos, fragNormal);
  base *= bright;
  // Linear fog based on distance
  float dist = length(fragWorldPos - cameraPos);
  float f = clamp((fogEnd - dist) / max(fogEnd - fogStart, 0.0001), 0.0, 1.0);
  vec3 rgb = mix(fogColor, base, f);
  if (underwater > 0) {
    rgb = mix(fogColor, rgb, 0.85);
  }
  // Leaves are treated as fully opaque; no alpha handling
  finalColor = vec4(rgb, 1.0);
}
