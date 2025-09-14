#version 330
in vec2 fragTexCoord;
in vec4 fragColor;
in vec3 fragWorldPos;
in vec3 fragNormal;
out vec4 finalColor;
uniform sampler2D texture0;
// Phase 2 lighting
uniform sampler2D lightTex;
uniform ivec3 lightDims;
uniform ivec2 lightGrid;
uniform vec3  chunkOrigin;
uniform float visualLightMin;
uniform float skyLightScale;
uniform vec3 fogColor;
uniform float fogStart;
uniform float fogEnd;
uniform vec3 cameraPos;
uniform float time;
uniform int underwater;

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
  // Interior dims exclude rings on X/Z
  vec3 p = worldPos - chunkOrigin;
  ivec3 innerDims = ivec3(lightDims.x - 2, lightDims.y, lightDims.z - 2);
  ivec3 vInner = ivec3(clamp(floor(p), vec3(0.0), vec3(innerDims) - vec3(1.0)));
  ivec3 step = ivec3(0,0,0);
  if (abs(nrm.x) > abs(nrm.y) && abs(nrm.x) > abs(nrm.z)) {
    step.x = (nrm.x > 0.0) ? 1 : -1;
  } else if (abs(nrm.z) > abs(nrm.y)) {
    step.z = (nrm.z > 0.0) ? 1 : -1;
  } else {
    step.y = (nrm.y > 0.0) ? 1 : -1;
  }
  // Allow sampling of both -X/-Z and +X/+Z atlas rings by clamping up to innerDims (inclusive)
  ivec3 vnInner = clamp(vInner + step, ivec3(-1), innerDims);
  ivec3 vAtlas = vInner + ivec3(1, 0, 1);
  ivec3 vnAtlas = vnInner + ivec3(1, 0, 1);
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
  // Subtle UV distortion based on world position and time
  float wave = sin(fragWorldPos.x * 0.15 + time * 0.8) * 0.01 + cos(fragWorldPos.z * 0.12 - time * 0.6) * 0.01;
  vec2 uv = fragTexCoord + vec2(wave, wave);
  vec4 base = texture(texture0, uv) * fragColor;
  // Apply light
  float bright = sampleBrightness(fragWorldPos, fragNormal);
  base.rgb *= bright;
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
