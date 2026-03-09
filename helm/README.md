# Development Notes

## Check that Kubernetes is reachable locally

```bash
kubectl get nodes
```

## Install the local Portabase Agent Helm chart

```bash
helm install portabase-agent .
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
helm upgrade --install portabase-agent .
```
