#version 330
in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;
uniform sampler2D texture0;
uniform vec2 resolution;

// Minimal FXAA 3.11-inspired pass (single step)
vec3 sampleCol(vec2 uv){ return texture(texture0, uv).rgb; }
void main(){
  vec2 inv = 1.0 / resolution;
  vec3 rgbM = sampleCol(fragTexCoord);
  vec3 rgbNW = sampleCol(fragTexCoord + inv * vec2(-1.0,-1.0));
  vec3 rgbNE = sampleCol(fragTexCoord + inv * vec2( 1.0,-1.0));
  vec3 rgbSW = sampleCol(fragTexCoord + inv * vec2(-1.0, 1.0));
  vec3 rgbSE = sampleCol(fragTexCoord + inv * vec2( 1.0, 1.0));
  vec3 lumaW = vec3(0.299, 0.587, 0.114);
  float lumaM = dot(rgbM, lumaW);
  float lumaNW = dot(rgbNW, lumaW);
  float lumaNE = dot(rgbNE, lumaW);
  float lumaSW = dot(rgbSW, lumaW);
  float lumaSE = dot(rgbSE, lumaW);
  float lumaMin = min(lumaM, min(min(lumaNW,lumaNE), min(lumaSW,lumaSE)));
  float lumaMax = max(lumaM, max(max(lumaNW,lumaNE), max(lumaSW,lumaSE)));
  vec2 dir;
  dir.x = -((lumaNW + lumaNE) - (lumaSW + lumaSE));
  dir.y =  ((lumaNW + lumaSW) - (lumaNE + lumaSE));
  float dirReduce = max((lumaNW + lumaNE + lumaSW + lumaSE) * 0.25 * 0.0312, 0.0078125);
  float rcpDirMin = 1.0 / (min(abs(dir.x), abs(dir.y)) + dirReduce);
  dir = clamp(dir * rcpDirMin, vec2(-8.0), vec2(8.0)) * inv;
  vec3 rgbA = 0.5 * ( sampleCol(fragTexCoord + dir * (1.0/3.0 - 0.5))
                    + sampleCol(fragTexCoord + dir * (2.0/3.0 - 0.5)) );
  vec3 rgbB = rgbA * 0.5 + 0.25 * ( sampleCol(fragTexCoord + dir * -0.5)
                                   + sampleCol(fragTexCoord + dir *  0.5) );
  float lumaB = dot(rgbB, lumaW);
  vec3 aa = (lumaB < lumaMin || lumaB > lumaMax) ? rgbA : rgbB;
  finalColor = vec4(aa, 1.0) * fragColor;
}

