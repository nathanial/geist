#version 330
in vec3 vertexPosition;
in vec2 vertexTexCoord;
in vec4 vertexColor;
out vec2 fragTexCoord;
out vec4 fragColor;
out vec3 fragWorldPos;
uniform mat4 mvp;
uniform mat4 matModel; // provided by raylib per draw (model transform)
void main(){
  fragTexCoord = vertexTexCoord;
  fragColor = vertexColor;
  // Compute true world-space position using the current model transform.
  // This fixes fog distance for meshes drawn with a translation (e.g., structures).
  fragWorldPos = (matModel * vec4(vertexPosition, 1.0)).xyz;
  gl_Position = mvp * vec4(vertexPosition, 1.0);
}
