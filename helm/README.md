# Development Notes

## Check that Kubernetes is reachable locally

```bash
kubectl get nodes
```

## Install the local Portabase Agent Helm chart

```bash
helm install portabase-agent . \ 
--set env.EDGE_KEY=<your-edge-key>
```

## Check the pods

```bash
kubectl get pods
```

## Check the services

```bash
kubectl get svc
```

## To update .env variables or JSON config:
```bash
kubectl rollout restart deployment portabase-agent
```

## Install or upgrade the Helm chart
```bash
helm upgrade portabase-agent . \
--reuse-values \
--set env.EDGE_KEY="NEW_EDGE_KEY"
```

## Rollout  to restart
```bash 
kubectl rollout restart deployment portabase-agent
```

## List pods to get the pod name 

```bash
kubectl get pods -l app=portabase-agent
```

## Get logs for the pod
```bash
kubectl logs portabase-agent-6f7d4f5c6b-abc12
```

## Uninstall Agent
``` bash
helm uninstall portabase-agent
```
