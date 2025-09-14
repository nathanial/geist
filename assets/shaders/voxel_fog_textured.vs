#version 330
in vec3 vertexPosition;
in vec2 vertexTexCoord;
in vec4 vertexColor;
in vec3 vertexNormal;
out vec2 fragTexCoord;
out vec4 fragColor;
out vec3 fragWorldPos;
out vec3 fragNormal;
uniform mat4 mvp;
uniform mat4 matModel; // provided by raylib per draw (model transform)
void main(){
  fragTexCoord = vertexTexCoord;
  fragColor = vertexColor;
  // Compute true world-space position using the current model transform.
  fragWorldPos = (matModel * vec4(vertexPosition, 1.0)).xyz;
  // Normal in world space (model assumed rotationless or uniform scale for chunks)
  fragNormal = normalize((mat3(matModel) * vertexNormal));
  gl_Position = mvp * vec4(vertexPosition, 1.0);
}
