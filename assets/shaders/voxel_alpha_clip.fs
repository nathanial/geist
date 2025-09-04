#version 330
in vec2 fragTexCoord;
in vec4 fragColor;
out vec4 finalColor;
uniform sampler2D texture0;
uniform float alphaCutoff;
void main(){
  vec4 tex = texture(texture0, fragTexCoord);
  if (tex.a < alphaCutoff) discard;
  finalColor = vec4(tex.rgb, tex.a) * fragColor;
}

